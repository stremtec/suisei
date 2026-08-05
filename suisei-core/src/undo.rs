//! Delta-based undo: the log stores [`Delta`]s — char-offset changes that
//! carry their own inverse description — never full-buffer snapshots.
//! Undo/redo apply deltas straight to the buffer via [`Buffer::apply_edit`].
//!
//! Memory model:
//! - only the newest [`IN_RAM_MAX`] deltas stay in RAM; older ones spill to
//!   `~/.suisei/undo/<fnv(path)>.undo` and stream back in on deep undo
//! - `undo_caching = true` keeps the spill file on close (plus a `.meta`
//!   content hash) so reopening the same, unchanged file resumes its history;
//!   `false` (default) deletes it
//!
//! The push path still diffs consecutive checkpoints into a delta (the edit
//! paths record checkpoints, not raw edits — yet). The diff is line-granular
//! but stored as a char-offset [`Change`], so undo already applies through
//! the same `apply_edit` the native edit migration will use; swapping the
//! push path to edit-produced deltas later changes nothing downstream.

use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

use crate::buffer::{Buffer, BufferSnapshot, Position};
use crate::edit::{Change, Delta, Edit};

/// Newest deltas kept in RAM; older ones go to the spill file.
pub const IN_RAM_MAX: usize = 50;
/// Safety cap for unnamed buffers (no spill target): drop oldest beyond this.
const NO_SPILL_MAX: usize = 500;

#[derive(Clone, Default)]
pub struct UndoStack {
    past: Vec<Delta>,
    future: Vec<Delta>,
    /// Anchor: the most recent checkpoint (Arc → cheap tab clones).
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

    /// Checkpoint the state after an edit. Consecutive checkpoints diff into
    /// one delta each; identical states are discarded (no wasted slots for
    /// `i`+Esc no-typing).
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

    /// Undo: absorb any uncommitted tail (edits in flight when undo is hit),
    /// then apply one delta's inverse straight to the buffer. Restores the
    /// pre-edit cursor.
    pub fn undo(&mut self, buffer: &mut Buffer) -> bool {
        self.absorb_tail(&buffer.snapshot());
        let delta = match self.past.pop() {
            Some(d) => d,
            None => match self.unspill_one() {
                Some(d) => d,
                None => return false,
            },
        };
        buffer.apply_edit(&delta.inverse());
        buffer.cursor = delta.cursor_before;
        // The anchor must follow the buffer: without this, the next undo's
        // absorb_tail diffs against a stale state and re-captures the edit
        // this undo just removed (undo runaway).
        self.last = Some(std::sync::Arc::new(buffer.snapshot()));
        self.future.push(delta);
        true
    }

    /// Redo the most recently undone delta (re-applied forward).
    pub fn redo(&mut self, buffer: &mut Buffer) -> bool {
        let Some(delta) = self.future.pop() else {
            return false;
        };
        let edit = Edit {
            changes: delta.changes.clone(),
        };
        buffer.apply_edit(&edit);
        buffer.cursor = delta.cursor_after;
        self.last = Some(std::sync::Arc::new(buffer.snapshot()));
        self.past.push(delta);
        self.spill_overflow();
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty() || !self.spill_offsets.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// The buffer changed after the last checkpoint without another push —
    /// capture that edit so it undoes first.
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
    fn unspill_one(&mut self) -> Option<Delta> {
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
            let drained: Vec<Delta> = std::mem::take(&mut self.past);
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

// ── Diff / convert ─────────────────────────────────────────────────────────

/// Line-range diff via common prefix/suffix trim, stored as a char-offset
/// delta. None when identical.
fn diff_snapshots(a: &BufferSnapshot, b: &BufferSnapshot) -> Option<Delta> {
    let (start, old, new) = diff_lines(a.lines(), b.lines())?;
    let change = line_delta_to_change(start, old, new, a.lines());
    Some(Delta {
        version_before: a.version(),
        version_after: b.version(),
        changes: vec![change],
        cursor_before: a.cursor(),
        cursor_after: b.cursor(),
    })
}

/// Common prefix/suffix trim → (start line, old lines, new lines). None when
/// identical. Shared with the LSP incremental sync.
pub(crate) fn diff_lines<'a>(
    al: &'a [String],
    bl: &'a [String],
) -> Option<(usize, &'a [String], &'a [String])> {
    let mut start = 0;
    let max_start = al.len().min(bl.len());
    while start < max_start && al[start] == bl[start] {
        start += 1;
    }
    if start == al.len() && start == bl.len() {
        return None;
    }
    let mut a_end = al.len();
    let mut b_end = bl.len();
    while a_end > start && b_end > start && al[a_end - 1] == bl[b_end - 1] {
        a_end -= 1;
        b_end -= 1;
    }
    Some((start, &al[start..a_end], &bl[start..b_end]))
}

/// Line-range replacement → one char-offset [`Change`] against `before`.
///
/// Lines are stored without their terminators, so a line range maps to a
/// char range plus exactly one boundary newline, chosen so the round trip
/// is exact: replacements keep the separator after the range; deletions
/// consume one separator (the trailing one mid-document, the leading one
/// when the tail is removed); insertions bring their own separator.
pub(crate) fn line_delta_to_change(
    start_line: usize,
    old_lines: &[String],
    new_lines: &[String],
    before: &[String],
) -> Change {
    let start = line_off(before, start_line);
    let old = old_lines.join("\n");
    let new = new_lines.join("\n");

    if old_lines.is_empty() && !new_lines.is_empty() {
        // Pure line insertion: the inserted lines carry their own separator
        // — trailing mid-document, leading at the document end.
        let new = if start_line >= before.len() {
            format!("\n{new}")
        } else {
            format!("{new}\n")
        };
        return Change { start, old, new };
    }
    if new_lines.is_empty() && !old_lines.is_empty() {
        // Pure line deletion: consume one separator too. At the document
        // tail that is the LEADING newline (there is no trailing one).
        let (start, old) = if start_line + old_lines.len() >= before.len() && start_line > 0 {
            (start.saturating_sub(1), format!("\n{old}"))
        } else {
            (start, format!("{old}\n"))
        };
        return Change { start, old, new };
    }
    // Replacement: the separator after the range stays put.
    Change { start, old, new }
}

/// Char offset of the start of `row` (== document end when row >= len).
fn line_off(lines: &[String], row: usize) -> usize {
    if row >= lines.len() {
        return lines.iter().map(|l| l.chars().count()).sum::<usize>()
            + lines.len().saturating_sub(1);
    }
    lines.iter().take(row).map(|l| l.chars().count() + 1).sum()
}

// ── Spill file format ──────────────────────────────────────────────────────
//
// Binary, length-prefixed (change text may contain anything):
//   [u64 version_before][u64 version_after][u32 n_changes]
//   per change: [u64 start][u64 old_len][old bytes][u64 new_len][new bytes]
//   [u32 cursor_before.row][u32 .col][u32 cursor_after.row][u32 .col]

fn append_record(path: &Path, d: &Delta) -> Option<u64> {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()?;
    let off = f.metadata().ok()?.len();
    let mut buf = Vec::new();
    buf.extend_from_slice(&d.version_before.to_le_bytes());
    buf.extend_from_slice(&d.version_after.to_le_bytes());
    buf.extend_from_slice(&(d.changes.len() as u32).to_le_bytes());
    for c in &d.changes {
        buf.extend_from_slice(&(c.start as u64).to_le_bytes());
        buf.extend_from_slice(&(c.old.len() as u64).to_le_bytes());
        buf.extend_from_slice(c.old.as_bytes());
        buf.extend_from_slice(&(c.new.len() as u64).to_le_bytes());
        buf.extend_from_slice(c.new.as_bytes());
    }
    buf.extend_from_slice(&(d.cursor_before.row as u32).to_le_bytes());
    buf.extend_from_slice(&(d.cursor_before.col as u32).to_le_bytes());
    buf.extend_from_slice(&(d.cursor_after.row as u32).to_le_bytes());
    buf.extend_from_slice(&(d.cursor_after.col as u32).to_le_bytes());
    f.write_all(&buf).ok()?;
    Some(off)
}

fn read_record_at(path: &Path, off: u64) -> Option<Delta> {
    let mut f = std::fs::File::open(path).ok()?;
    f.seek(std::io::SeekFrom::Start(off)).ok()?;
    let mut h = [0u8; 20];
    f.read_exact(&mut h).ok()?;
    let vb = u64::from_le_bytes(h[0..8].try_into().ok()?);
    let va = u64::from_le_bytes(h[8..16].try_into().ok()?);
    let n = u32::from_le_bytes(h[16..20].try_into().ok()?) as usize;
    let mut changes = Vec::with_capacity(n);
    for _ in 0..n {
        let mut m = [0u8; 16];
        f.read_exact(&mut m).ok()?;
        let start = u64::from_le_bytes(m[0..8].try_into().ok()?) as usize;
        let old_len = u64::from_le_bytes(m[8..16].try_into().ok()?) as usize;
        let mut old_bytes = vec![0u8; old_len];
        f.read_exact(&mut old_bytes).ok()?;
        let mut nl = [0u8; 8];
        f.read_exact(&mut nl).ok()?;
        let new_len = u64::from_le_bytes(nl) as usize;
        let mut new_bytes = vec![0u8; new_len];
        f.read_exact(&mut new_bytes).ok()?;
        changes.push(Change {
            start,
            old: String::from_utf8(old_bytes).ok()?,
            new: String::from_utf8(new_bytes).ok()?,
        });
    }
    let mut c = [0u8; 16];
    f.read_exact(&mut c).ok()?;
    let cor = u32::from_le_bytes(c[0..4].try_into().ok()?) as usize;
    let coc = u32::from_le_bytes(c[4..8].try_into().ok()?) as usize;
    let cnr = u32::from_le_bytes(c[8..12].try_into().ok()?) as usize;
    let cnc = u32::from_le_bytes(c[12..16].try_into().ok()?) as usize;
    Some(Delta {
        version_before: vb,
        version_after: va,
        changes,
        cursor_before: Position::new(cor, coc),
        cursor_after: Position::new(cnr, cnc),
    })
}

/// Offsets of every record in an existing spill file (resume path).
fn scan_offsets(path: &Path) -> Vec<u64> {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let mut offsets = Vec::new();
    loop {
        let off = match f.stream_position() {
            Ok(o) => o,
            Err(_) => break,
        };
        let mut h = [0u8; 20];
        if f.read_exact(&mut h).is_err() {
            break; // clean EOF or corrupt tail — keep what parsed
        }
        let n = u32::from_le_bytes(h[16..20].try_into().unwrap_or([0; 4])) as usize;
        let mut ok = true;
        for _ in 0..n {
            // Record order is [start][old_len][old bytes][new_len][new
            // bytes] — skip in exactly that order.
            let mut m = [0u8; 16];
            if f.read_exact(&mut m).is_err() {
                ok = false;
                break;
            }
            let old_len = u64::from_le_bytes(m[8..16].try_into().unwrap_or([0; 8]));
            if f.seek(std::io::SeekFrom::Current(old_len as i64)).is_err() {
                ok = false;
                break;
            }
            let mut nl = [0u8; 8];
            if f.read_exact(&mut nl).is_err() {
                ok = false;
                break;
            }
            let new_len = u64::from_le_bytes(nl);
            if f.seek(std::io::SeekFrom::Current(new_len as i64)).is_err() {
                ok = false;
                break;
            }
        }
        if !ok {
            break;
        }
        let mut c = [0u8; 16];
        if f.read_exact(&mut c).is_err() {
            break;
        }
        offsets.push(off);
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
    crate::fs_atomic::state_path("undo")
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

    fn buf(text: &str, row: usize, col: usize) -> Buffer {
        let mut b = Buffer::from_string(text);
        b.cursor = Position::new(row, col);
        b
    }

    /// undo restores `before` (and its cursor), redo restores `after`.
    fn undo_redo(before: &str, after: &str) {
        let mut u = UndoStack::new();
        let mut b = buf(before, 0, 0);
        u.push(b.snapshot());
        b = buf(after, 0, 0);
        assert!(u.undo(&mut b), "undo failed: {before:?} → {after:?}");
        assert_eq!(
            b.text(),
            before,
            "undo must restore {before:?} (from {after:?})"
        );
        assert!(u.redo(&mut b), "redo failed: {before:?} → {after:?}");
        assert_eq!(b.text(), after, "redo must restore {after:?}");
    }

    #[test]
    fn delta_roundtrip_single_line() {
        let mut u = UndoStack::new();
        let mut b = buf("alpha\nbeta\ngamma", 1, 0);
        u.push(b.snapshot());
        b = buf("alpha\nBETA\ngamma", 1, 4);
        assert!(u.undo(&mut b));
        assert_eq!(b.text(), "alpha\nbeta\ngamma");
        assert_eq!(b.cursor, Position::new(1, 0), "pre-edit cursor restored");
        assert!(u.redo(&mut b));
        assert_eq!(b.text(), "alpha\nBETA\ngamma");
        assert_eq!(b.cursor, Position::new(1, 4), "post-edit cursor restored");
    }

    #[test]
    fn noop_push_consumes_nothing() {
        let mut u = UndoStack::new();
        let b = buf("x", 0, 0);
        u.push(b.snapshot());
        u.push(b.snapshot()); // no typing between checkpoints
        u.push(b.snapshot());
        assert!(!u.can_undo());
    }

    #[test]
    fn insert_and_delete_lines() {
        let mut u = UndoStack::new();
        let mut b = buf("a\nb", 0, 0);
        u.push(b.snapshot());
        b = buf("a\nnew1\nnew2\nb", 2, 0);
        u.push(b.snapshot()); // growth committed
        b = buf("a", 0, 0); // uncommitted shrink
        assert!(u.undo(&mut b), "undo shrink");
        assert_eq!(b.text(), "a\nnew1\nnew2\nb");
        assert!(u.undo(&mut b), "undo growth");
        assert_eq!(b.text(), "a\nb");
        assert!(!u.can_undo());
    }

    /// The newline-boundary rules, one per shape of line-range edit.
    #[test]
    fn line_delta_edge_cases() {
        undo_redo("l1\nl2\nl3", "l1\nx\nl3"); // mid replace
        undo_redo("l1\nl2\nl3", "l1\nl2\nx"); // tail replace
        undo_redo("l1\nl2\nl3", "l1\nl3"); // mid delete
        undo_redo("l1\nl2\nl3", "l1\nl2"); // tail delete
        undo_redo("l1\nl2", "l1\nx\nl2"); // mid insert
        undo_redo("l1\nl2", "l1\nl2\nx"); // tail insert
        undo_redo("l1\nl2\nl3", "x"); // replace all
        undo_redo("l1", "l1\na\nb"); // one line grows
        undo_redo("l1\nl2\nl3", "l1"); // delete tail lines
        undo_redo("a\nb\nc\nd", "a\nd"); // delete middle lines
    }

    #[test]
    fn spill_and_deep_undo() {
        let dir = std::env::temp_dir().join(format!("suisei-undo-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("doc.txt");
        std::fs::write(&file, "seed").unwrap();

        let mut u = UndoStack::new();
        u.attach_file(&file, false, "seed");
        // 80 edits → 30 must spill to disk (IN_RAM_MAX = 50).
        let mut text = String::from("line0");
        let mut b = buf(&text, 0, 0);
        u.push(b.snapshot());
        for i in 1..=80 {
            let next = format!("{text}\nline{i}");
            b = buf(&next, 0, 0);
            u.push(b.snapshot());
            text = next;
        }
        let mut steps = 0;
        while u.undo(&mut b) {
            steps += 1;
            if steps > 200 {
                panic!("undo runaway");
            }
        }
        assert_eq!(b.text(), "line0");
        assert_eq!(steps, 80);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_and_resume() {
        let dir = std::env::temp_dir().join(format!("suisei-undo-res-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("doc.txt");
        std::fs::write(&file, "v2").unwrap();

        let mut u = UndoStack::new();
        u.attach_file(&file, true, "v2");
        let mut b = buf("v1", 0, 0);
        u.push(b.snapshot());
        b = buf("v2", 0, 0);
        u.push(b.snapshot()); // delta v1→v2 committed
        u.finish(true, "v2");

        // Reopen same content → history resumes from disk.
        let mut u2 = UndoStack::new();
        u2.attach_file(&file, true, "v2");
        assert!(u2.can_undo(), "cached history should resume");
        let mut b2 = buf("v2", 0, 0);
        assert!(u2.undo(&mut b2), "undo from cache");
        assert_eq!(b2.text(), "v1");

        // Changed content → cache invalidated.
        let mut u3 = UndoStack::new();
        u3.attach_file(&file, true, "v2-changed-outside");
        assert!(!u3.can_undo(), "stale cache must be dropped");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finish_without_caching_removes_spill() {
        let dir = std::env::temp_dir().join(format!("suisei-undo-rm-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("doc.txt");
        std::fs::write(&file, "x").unwrap();
        let mut u = UndoStack::new();
        u.attach_file(&file, false, "x");
        let mut text = String::from("l0");
        let mut b = buf(&text, 0, 0);
        u.push(b.snapshot());
        for i in 1..=60 {
            let next = format!("{text}\nl{i}");
            b = buf(&next, 0, 0);
            u.push(b.snapshot());
            text = next;
        }
        let spill = spill_file_for(&file);
        assert!(spill.exists(), "overflow should have spilled");
        u.finish(false, &text);
        assert!(!spill.exists(), "no-caching close must clean up");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
