//! Delta-based undo with SSD spill + optional on-close persistence.
//!
//! Memory model (replaces full-buffer snapshots that grew O(edits × file)):
//! - each history entry is a **line-range delta** (changed lines only)
//! - one full snapshot (`last`, Arc-shared) anchors the live end of the chain
//! - only the newest [`IN_RAM_MAX`] deltas stay in RAM; older ones spill to
//!   `~/.xei/undo/<fnv(path)>.undo` and stream back in on deep undo
//! - `undo_caching = true` keeps the spill file on close (plus a `.meta`
//!   content hash) so reopening the same, unchanged file resumes its history;
//!   `false` (default) deletes it
//!
//! The public API mirrors the old snapshot stack (`push` the pre-edit state,
//! `undo/redo` exchange full snapshots) so call sites stay untouched.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::buffer::{BufferSnapshot, Position};

/// Newest deltas kept in RAM; older ones go to the spill file.
pub const IN_RAM_MAX: usize = 50;
/// Safety cap for unnamed buffers (no spill target): drop oldest beyond this.
const NO_SPILL_MAX: usize = 500;

/// One edit as a reversible line-range patch.
#[derive(Clone, Debug)]
struct EditDelta {
    /// First differing line index.
    start: usize,
    /// Lines this range held *before* the edit (apply to undo).
    old: Vec<String>,
    /// Lines this range holds *after* the edit (apply to redo).
    new: Vec<String>,
    cursor_old: Position,
    cursor_new: Position,
}

#[derive(Clone, Default)]
pub struct UndoStack {
    past: Vec<EditDelta>,
    future: Vec<EditDelta>,
    /// Anchor: the most recent state the stack has seen (Arc → cheap tab clones).
    last: Option<std::sync::Arc<BufferSnapshot>>,
    /// Spill file for entries beyond IN_RAM_MAX (None for unnamed buffers).
    spill_path: Option<PathBuf>,
    /// Byte offset of each spilled record (oldest → newest).
    spill_offsets: Vec<u64>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the state *before* a mutating edit. Consecutive pushes diff into
    /// a delta; identical states are discarded (no more wasted `i`+Esc slots).
    pub fn push(&mut self, snapshot: BufferSnapshot) {
        if let Some(prev) = self.last.clone() {
            if let Some(delta) = diff_snapshots(&prev, &snapshot) {
                self.past.push(delta);
                self.future.clear();
                self.spill_overflow();
            }
        }
        self.last = Some(std::sync::Arc::new(snapshot));
    }

    /// Undo: absorb any uncommitted edit, then walk one delta back.
    pub fn undo(&mut self, current: BufferSnapshot) -> Option<BufferSnapshot> {
        self.absorb_tail(&current);
        let delta = match self.past.pop() {
            Some(d) => d,
            None => self.unspill_one()?,
        };
        let prev = apply_delta(&current, &delta, false);
        self.future.push(delta);
        self.last = Some(std::sync::Arc::new(prev.clone()));
        Some(prev)
    }

    /// Redo the most recently undone delta.
    pub fn redo(&mut self, current: BufferSnapshot) -> Option<BufferSnapshot> {
        let delta = self.future.pop()?;
        let next = apply_delta(&current, &delta, true);
        self.past.push(delta);
        self.spill_overflow();
        self.last = Some(std::sync::Arc::new(next.clone()));
        Some(next)
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty() || !self.spill_offsets.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// The buffer changed after the last `push` without another push (edits in
    /// flight when `u` is hit) — capture that edit so it undoes first.
    fn absorb_tail(&mut self, current: &BufferSnapshot) {
        if let Some(prev) = self.last.clone() {
            if let Some(delta) = diff_snapshots(&prev, current) {
                self.past.push(delta);
                self.future.clear();
                self.spill_overflow();
            }
        }
        self.last = Some(std::sync::Arc::new(current.clone()));
    }

    // ── Spill: oldest deltas move to disk ──────────────────────────────

    /// Bind this stack to a file (spill target). Optionally resume a cached
    /// history when the on-disk content hash still matches `text`.
    pub fn attach_file(&mut self, path: &Path, caching: bool, text: &str) {
        let spill = spill_file_for(path);
        self.spill_path = Some(spill.clone());
        self.spill_offsets.clear();
        if caching && meta_matches(&spill, text) {
            self.spill_offsets = scan_offsets(&spill);
        } else {
            let _ = std::fs::remove_file(&spill);
            let _ = std::fs::remove_file(meta_path(&spill));
        }
    }

    fn spill_overflow(&mut self) {
        if self.past.len() <= IN_RAM_MAX {
            return;
        }
        let Some(path) = self.spill_path.clone() else {
            // Unnamed buffer — keep a hard cap instead of unbounded RAM.
            while self.past.len() > NO_SPILL_MAX {
                self.past.remove(0);
            }
            return;
        };
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
        while self.past.len() > IN_RAM_MAX {
            let oldest = self.past.remove(0);
            if let Some(off) = append_record(&path, &oldest) {
                self.spill_offsets.push(off);
            }
        }
    }

    /// Pull the newest spilled record back off disk.
    fn unspill_one(&mut self) -> Option<EditDelta> {
        let path = self.spill_path.clone()?;
        let off = self.spill_offsets.pop()?;
        let delta = read_record_at(&path, off)?;
        // Truncate so the file stays a clean stack.
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&path) {
            let _ = f.set_len(off);
        }
        Some(delta)
    }

    /// File is closing: persist the whole history (undo_caching = true) or
    /// remove the session spill (false).
    pub fn finish(&mut self, caching: bool, text: &str) {
        let Some(path) = self.spill_path.clone() else {
            return;
        };
        if caching {
            let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
            let drained: Vec<EditDelta> = std::mem::take(&mut self.past);
            for d in drained {
                if let Some(off) = append_record(&path, &d) {
                    self.spill_offsets.push(off);
                }
            }
            write_meta(&path, text);
        } else {
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(meta_path(&path));
        }
    }
}

// ── Diff / apply ───────────────────────────────────────────────────────────

/// Line-range diff via common prefix/suffix trim. None when identical.
fn diff_snapshots(a: &BufferSnapshot, b: &BufferSnapshot) -> Option<EditDelta> {
    let (al, bl) = (a.lines(), b.lines());
    let mut start = 0;
    let max_start = al.len().min(bl.len());
    while start < max_start && al[start] == bl[start] {
        start += 1;
    }
    if start == al.len() && start == bl.len() {
        return None; // identical content
    }
    let mut a_end = al.len();
    let mut b_end = bl.len();
    while a_end > start && b_end > start && al[a_end - 1] == bl[b_end - 1] {
        a_end -= 1;
        b_end -= 1;
    }
    Some(EditDelta {
        start,
        old: al[start..a_end].to_vec(),
        new: bl[start..b_end].to_vec(),
        cursor_old: a.cursor(),
        cursor_new: b.cursor(),
    })
}

/// Rebuild the neighbouring state from `current` and a delta.
fn apply_delta(current: &BufferSnapshot, d: &EditDelta, forward: bool) -> BufferSnapshot {
    let (replace_with, expect_len, cursor) = if forward {
        (&d.new, d.old.len(), d.cursor_new)
    } else {
        (&d.old, d.new.len(), d.cursor_old)
    };
    let mut lines = current.lines().to_vec();
    let end = (d.start + expect_len).min(lines.len());
    let start = d.start.min(lines.len());
    lines.splice(start..end, replace_with.iter().cloned());
    if lines.is_empty() {
        lines.push(String::new());
    }
    BufferSnapshot::from_parts(lines, cursor)
}

// ── Spill file format ──────────────────────────────────────────────────────
//
// Buffer lines never contain `\n`, so a line-oriented record is unambiguous:
//   @ <start> <n_old> <n_new> <cor> <coc> <cnr> <cnc>
//   …n_old old lines…
//   …n_new new lines…

fn append_record(path: &Path, d: &EditDelta) -> Option<u64> {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()?;
    let off = f.metadata().ok()?.len();
    let mut buf = format!(
        "@ {} {} {} {} {} {} {}\n",
        d.start,
        d.old.len(),
        d.new.len(),
        d.cursor_old.row,
        d.cursor_old.col,
        d.cursor_new.row,
        d.cursor_new.col
    );
    for l in &d.old {
        buf.push_str(l);
        buf.push('\n');
    }
    for l in &d.new {
        buf.push_str(l);
        buf.push('\n');
    }
    f.write_all(buf.as_bytes()).ok()?;
    Some(off)
}

fn read_record_at(path: &Path, off: u64) -> Option<EditDelta> {
    let data = std::fs::read_to_string(path).ok()?;
    let rec = data.get(off as usize..)?;
    let mut it = rec.lines();
    let header = it.next()?;
    let mut h = header.strip_prefix("@ ")?.split_whitespace();
    let start: usize = h.next()?.parse().ok()?;
    let n_old: usize = h.next()?.parse().ok()?;
    let n_new: usize = h.next()?.parse().ok()?;
    let cor: usize = h.next()?.parse().ok()?;
    let coc: usize = h.next()?.parse().ok()?;
    let cnr: usize = h.next()?.parse().ok()?;
    let cnc: usize = h.next()?.parse().ok()?;
    let mut old = Vec::with_capacity(n_old);
    for _ in 0..n_old {
        old.push(it.next()?.to_string());
    }
    let mut new = Vec::with_capacity(n_new);
    for _ in 0..n_new {
        new.push(it.next()?.to_string());
    }
    Some(EditDelta {
        start,
        old,
        new,
        cursor_old: Position::new(cor, coc),
        cursor_new: Position::new(cnr, cnc),
    })
}

/// Offsets of every record in an existing spill file (resume path).
fn scan_offsets(path: &Path) -> Vec<u64> {
    let Ok(data) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut offsets = Vec::new();
    let mut off = 0u64;
    let mut lines = data.split_inclusive('\n');
    while let Some(header) = lines.next() {
        let Some(h) = header.trim_end().strip_prefix("@ ") else {
            break; // corrupt tail — ignore the rest
        };
        let mut parts = h.split_whitespace();
        let (Some(_), Some(n_old), Some(n_new)) =
            (parts.next(), parts.next(), parts.next())
        else {
            break;
        };
        let (Ok(n_old), Ok(n_new)) = (n_old.parse::<usize>(), n_new.parse::<usize>())
        else {
            break;
        };
        offsets.push(off);
        off += header.len() as u64;
        for _ in 0..(n_old + n_new) {
            match lines.next() {
                Some(l) => off += l.len() as u64,
                None => return offsets, // truncated body — keep what parsed
            }
        }
    }
    offsets
}

// ── Cache identity ─────────────────────────────────────────────────────────

fn fnv64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn undo_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".xei").join("undo")
}

fn spill_file_for(path: &Path) -> PathBuf {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    undo_dir().join(format!("{:016x}.undo", fnv64(&abs.display().to_string())))
}

fn meta_path(spill: &Path) -> PathBuf {
    spill.with_extension("meta")
}

fn write_meta(spill: &Path, text: &str) {
    let _ = std::fs::write(meta_path(spill), format!("v1 {:016x}\n", fnv64(text)));
}

/// Cached history is only valid while the file content is unchanged.
fn meta_matches(spill: &Path, text: &str) -> bool {
    let Ok(meta) = std::fs::read_to_string(meta_path(spill)) else {
        return false;
    };
    meta.trim() == format!("v1 {:016x}", fnv64(text))
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;

    fn snap(text: &str, row: usize, col: usize) -> BufferSnapshot {
        let mut b = Buffer::from_string(text);
        b.cursor = Position::new(row, col);
        b.snapshot()
    }

    #[test]
    fn delta_roundtrip_single_line() {
        let mut u = UndoStack::new();
        u.push(snap("alpha\nbeta\ngamma", 1, 0)); // initial
        u.push(snap("alpha\nbeta\ngamma", 1, 0)); // pre-edit (same → no delta)
        // edit happened: beta → BETA
        let cur = snap("alpha\nBETA\ngamma", 1, 4);
        let back = u.undo(cur.clone()).expect("undo");
        assert_eq!(back.lines()[1], "beta");
        let fwd = u.redo(back).expect("redo");
        assert_eq!(fwd.lines()[1], "BETA");
    }

    #[test]
    fn noop_push_consumes_nothing() {
        let mut u = UndoStack::new();
        u.push(snap("x", 0, 0));
        u.push(snap("x", 0, 0)); // i + Esc, no typing
        u.push(snap("x", 0, 0));
        assert!(!u.can_undo());
    }

    #[test]
    fn insert_and_delete_lines() {
        let mut u = UndoStack::new();
        u.push(snap("a\nb", 0, 0));
        let grown = snap("a\nnew1\nnew2\nb", 2, 0);
        u.push(grown.clone()); // next pre-edit commits the growth delta
        let shrunk = snap("a", 0, 0);
        let mid = u.undo(shrunk).expect("undo shrink");
        assert_eq!(mid.lines(), grown.lines());
        let orig = u.undo(mid).expect("undo growth");
        assert_eq!(orig.lines(), ["a", "b"]);
        assert!(!u.can_undo());
    }

    #[test]
    fn spill_and_deep_undo() {
        let dir = std::env::temp_dir().join(format!("xei-undo-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("doc.txt");
        std::fs::write(&file, "seed").unwrap();

        let mut u = UndoStack::new();
        u.attach_file(&file, false, "seed");
        // 80 edits → 30 must spill to disk (IN_RAM_MAX = 50).
        let mut text = String::from("line0");
        u.push(snap(&text, 0, 0));
        for i in 1..=80 {
            let next = format!("{text}\nline{i}");
            u.push(snap(&next, 0, 0));
            text = next;
        }
        // absorb final edit then walk all 80 back.
        let mut cur = snap(&text, 0, 0);
        let mut steps = 0;
        while let Some(prev) = u.undo(cur.clone()) {
            cur = prev;
            steps += 1;
            if steps > 200 {
                panic!("undo runaway");
            }
        }
        assert_eq!(cur.lines(), ["line0"]);
        assert_eq!(steps, 80);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_and_resume() {
        let dir = std::env::temp_dir().join(format!("xei-undo-res-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("doc.txt");
        std::fs::write(&file, "v2").unwrap();

        let mut u = UndoStack::new();
        u.attach_file(&file, true, "v2");
        u.push(snap("v1", 0, 0));
        u.push(snap("v2", 0, 0)); // delta v1→v2 committed
        u.finish(true, "v2");

        // Reopen same content → history resumes from disk.
        let mut u2 = UndoStack::new();
        u2.attach_file(&file, true, "v2");
        assert!(u2.can_undo(), "cached history should resume");
        let back = u2.undo(snap("v2", 0, 0)).expect("undo from cache");
        assert_eq!(back.lines(), ["v1"]);

        // Changed content → cache invalidated.
        let mut u3 = UndoStack::new();
        u3.attach_file(&file, true, "v2-changed-outside");
        assert!(!u3.can_undo(), "stale cache must be dropped");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finish_without_caching_removes_spill() {
        let dir = std::env::temp_dir().join(format!("xei-undo-rm-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("doc.txt");
        std::fs::write(&file, "x").unwrap();
        let mut u = UndoStack::new();
        u.attach_file(&file, false, "x");
        let mut text = String::from("l0");
        u.push(snap(&text, 0, 0));
        for i in 1..=60 {
            let next = format!("{text}\nl{i}");
            u.push(snap(&next, 0, 0));
            text = next;
        }
        let spill = spill_file_for(&file);
        assert!(spill.exists(), "overflow should have spilled");
        u.finish(false, &text);
        assert!(!spill.exists(), "no-caching close must clean up");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
