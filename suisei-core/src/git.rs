//! Git gutter signs from `git diff` (working tree vs HEAD) + optional blame.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitSign {
    /// New line in working tree
    Added,
    /// Changed line
    Modified,
    /// Deletion adjacent to this line (show marker on the following line)
    Deleted,
}

/// One contiguous change against HEAD.
///
/// The unit every gutter interaction actually works on: the bar is drawn per
/// hunk, hovering highlights a hunk, and Stage / Discard / Show Change each
/// take one. The per-line [`GitSign`] map is DERIVED from these — it used to be
/// the only thing computed, which is why the gutter drew a separate 4pt-inset
/// stripe per line and a run of changed lines read as a dotted column rather
/// than one change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHunk {
    /// First line of the hunk in the CURRENT buffer, 0-based.
    pub start: usize,
    /// How many current-buffer lines it spans. Zero for a pure deletion, which
    /// occupies no line but is marked against `start`.
    pub len: usize,
    /// The lines this replaced, exactly as HEAD has them. Empty for a pure
    /// addition. This is what "Show Change" reveals above the hunk.
    pub removed: Vec<String>,
    pub kind: GitSign,
    /// This hunk's own slice of the diff, verbatim: the `@@` line and its
    /// body. Enough to build a one-hunk patch and hand it to `git apply`, so
    /// staging or discarding one change never has to re-derive what the change
    /// was — the bytes git produced are the bytes git gets back.
    pub patch: String,
    /// The change is in the index and not in the working tree's diff against
    /// it — `git add`ed and untouched since.
    ///
    /// Drives the bar's SHAPE, which is the distinction Xcode makes and the
    /// only place this state is visible without opening a panel: an unstaged
    /// hunk is a hollow outline, a staged one is filled solid.
    pub staged: bool,
}

impl GitHunk {
    /// Last current-buffer line the hunk covers, or `start` when it covers none.
    pub fn end(&self) -> usize {
        self.start + self.len.saturating_sub(1)
    }

    /// Whether `row` is inside the hunk. A pure deletion answers for the single
    /// line its marker sits on — there is nowhere else to hit it.
    pub fn contains(&self, row: usize) -> bool {
        if self.len == 0 {
            return row == self.start;
        }
        row >= self.start && row <= self.end()
    }
}

#[derive(Debug, Clone, Default)]
pub struct BlameLine {
    /// Short author (truncated)
    pub author: String,
    /// 7-char commit hash
    pub hash: String,
    /// Short date YYYY-MM-DD if known
    pub date: String,
}

/// Full blame column width when open (cells).
pub const BLAME_PANEL_WIDTH: u16 = 28;
/// Slide-open / slide-close duration (ms).
pub const BLAME_ANIM_MS: u64 = 300;

#[derive(Debug, Clone)]
pub struct GitBlame {
    /// 0-based line → blame info
    pub lines: HashMap<usize, BlameLine>,
    pub path: String,
    /// Panel open (or closing animation in progress).
    pub open: bool,
    pub available: bool,
    /// Legacy inline mode (`gb` line suffix) — kept for optional use.
    pub enabled: bool,
    // ── open animation (SCM-style openness) ──
    pub closing: bool,
    pub anim_from: f32,
    pub anim_to: f32,
    pub anim_pending: bool,
    pub opened_at: Option<std::time::Instant>,
}

impl Default for GitBlame {
    fn default() -> Self {
        Self {
            lines: HashMap::new(),
            path: String::new(),
            open: false,
            available: false,
            enabled: false,
            closing: false,
            anim_from: 0.0,
            anim_to: 0.0,
            anim_pending: false,
            opened_at: None,
        }
    }
}

impl GitBlame {
    pub fn clear(&mut self) {
        self.lines.clear();
        self.path.clear();
        self.available = false;
        self.enabled = false;
        self.open = false;
        self.closing = false;
        self.opened_at = None;
        self.anim_pending = false;
    }

    /// Whether the blame column should take layout space (open or animating).
    pub fn visible(&self) -> bool {
        self.open || self.closing
    }

    /// Linear openness 0..=1 for UI easing.
    pub fn anim_progress(&mut self) -> f32 {
        let v = self.tick_openness();
        if self.closing && v <= 0.001 {
            self.finish_close();
        }
        v
    }

    fn snapshot_openness(&self) -> f32 {
        if self.anim_pending {
            return self.anim_from;
        }
        let Some(t0) = self.opened_at else {
            return if self.open && !self.closing { 1.0 } else { 0.0 };
        };
        let u = (t0.elapsed().as_millis() as f32 / BLAME_ANIM_MS as f32).min(1.0);
        self.anim_from + (self.anim_to - self.anim_from) * u
    }

    fn tick_openness(&mut self) -> f32 {
        if self.anim_pending {
            self.anim_pending = false;
            self.opened_at = Some(std::time::Instant::now());
            return self.anim_from;
        }
        self.snapshot_openness()
    }

    fn finish_close(&mut self) {
        self.open = false;
        self.closing = false;
        self.enabled = false;
        self.opened_at = None;
        self.anim_pending = false;
        self.anim_from = 0.0;
        self.anim_to = 0.0;
    }

    /// Open panel with slide-in (loads blame for `path`).
    pub fn open_panel(&mut self, path: &str) -> String {
        // Optimistic open — `git blame` runs on a background thread and the
        // panel fills (or closes with a message) when the result lands.
        self.path = path.to_string();
        let from = if self.open || self.closing {
            self.snapshot_openness()
        } else {
            0.0
        };
        self.open = true;
        self.closing = false;
        self.enabled = true;
        self.anim_from = from;
        self.anim_to = 1.0;
        self.anim_pending = true;
        self.opened_at = None;
        if self.lines.is_empty() {
            "Blame · loading… · Ctrl+B close".into()
        } else {
            format!("Blame · {} lines · Ctrl+B close", self.lines.len())
        }
    }

    /// Slide-out close.
    pub fn close_panel(&mut self) {
        if !self.open || self.closing {
            if !self.open {
                self.enabled = false;
            }
            return;
        }
        let cur = self.snapshot_openness();
        self.closing = true;
        self.anim_from = cur;
        self.anim_to = 0.0;
        self.anim_pending = true;
        self.opened_at = None;
    }

    pub fn toggle_panel(&mut self, path: &str) -> String {
        if self.open && !self.closing {
            self.close_panel();
            "Blame closing…".into()
        } else if self.closing {
            // reopen mid-close
            self.open_panel(path)
        } else {
            self.open_panel(path)
        }
    }

    /// Legacy inline toggle (`gb`) — same panel for consistency.
    pub fn toggle(&mut self, path: &str) -> String {
        self.toggle_panel(path)
    }

    /// Sync wrapper (hot paths use App's async channel + `compute_blame`).
    pub fn refresh(&mut self, path: &str) {
        let (available, lines) = compute_blame(path);
        self.path = path.to_string();
        self.available = available;
        self.lines = lines;
    }

    pub fn at(&self, row: usize) -> Option<&BlameLine> {
        if self.enabled || self.open {
            self.lines.get(&row)
        } else {
            None
        }
    }
}

/// Fixed **flame** palette — independent of editor theme.

/// Blocking blame computation (runs on a background thread for the panel).
pub fn compute_blame(path: &str) -> (bool, HashMap<usize, BlameLine>) {
    let mut lines = HashMap::new();
    if path.is_empty() {
        return (false, lines);
    }
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| Path::new(path).to_path_buf());
    let Some(parent) = abs.parent() else {
        return (false, lines);
    };
    let output = Command::new("git")
        .args([
            "blame",
            "--line-porcelain",
            "--",
            abs.to_str().unwrap_or(path),
        ])
        .current_dir(parent)
        .output();
    let Ok(out) = output else {
        return (false, lines);
    };
    if !out.status.success() {
        return (false, lines);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    parse_blame_porcelain(&text, &mut lines);
    (!lines.is_empty(), lines)
}

pub fn flame_color_for(key: &str) -> (u8, u8, u8) {
    // Warm fire: deep red → orange → gold → ember
    const FLAME: &[(u8, u8, u8)] = &[
        (255, 48, 20),  // core red
        (255, 90, 25),  // orange-red
        (255, 130, 30), // orange
        (255, 170, 40), // amber
        (255, 200, 55), // gold
        (255, 110, 45), // ember
        (255, 70, 35),  // flame edge
        (255, 150, 60), // bright orange
    ];
    let h = key
        .bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(33).wrapping_add(b as u32));
    FLAME[(h as usize) % FLAME.len()]
}

/// Column width for current anim openness (eased in UI).
pub fn blame_width_for_openness(t: f32) -> u16 {
    let t = t.clamp(0.0, 1.0);
    ((BLAME_PANEL_WIDTH as f32) * t).round() as u16
}

/// Parse `git blame --line-porcelain` into per-line info.
pub fn parse_blame_porcelain(text: &str, out: &mut HashMap<usize, BlameLine>) {
    let mut hash = String::new();
    let mut author = String::new();
    let mut date = String::new();
    let mut line_no: Option<usize> = None; // 0-based final line

    for line in text.lines() {
        if line.len() >= 40
            && line
                .as_bytes()
                .get(0)
                .is_some_and(|b| b.is_ascii_hexdigit())
        {
            // header: hash orig final [group]
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                hash = parts[0].chars().take(7).collect();
                if let Ok(n) = parts[2].parse::<usize>() {
                    line_no = Some(n.saturating_sub(1));
                }
            }
            author.clear();
            date.clear();
        } else if let Some(a) = line.strip_prefix("author ") {
            author = a.chars().take(12).collect();
        } else if let Some(t) = line.strip_prefix("author-time ") {
            // unix timestamp → rough date via optional; keep raw short
            if let Ok(secs) = t.parse::<i64>() {
                // minimal YYYY without chrono: leave short stamp
                date = format!("{secs}");
                // Prefer author-mail skip; use time only if no better
            }
        } else if let Some(d) = line.strip_prefix("author-time ") {
            let _ = d;
        } else if line.starts_with('\t') {
            if let Some(row) = line_no {
                let auth = if author.is_empty() {
                    "?".into()
                } else {
                    author.clone()
                };
                out.insert(
                    row,
                    BlameLine {
                        author: auth,
                        hash: hash.clone(),
                        date: date.clone(),
                    },
                );
            }
            line_no = None;
        }
    }
}

/// Blocking gutter computation (runs on a background thread).
pub fn compute_gutter(path: &str) -> (bool, HashMap<usize, GitSign>, Vec<GitHunk>) {
    let mut signs = HashMap::new();
    if path.is_empty() {
        return (false, signs, Vec::new());
    }
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| Path::new(path).to_path_buf());
    let Some(parent) = abs.parent() else {
        return (false, signs, Vec::new());
    };
    // TWO diffs, because one cannot answer the question.
    //
    // `diff HEAD` is everything uncommitted — staged and unstaged together —
    // and its line numbers are the working tree's, which is what the gutter
    // needs. `diff` alone is the unstaged part, in the same coordinates. A
    // change present in the first and absent from the second has been staged.
    //
    // Doing it the other way round — asking `--cached` what is staged — looks
    // more direct and is wrong: `--cached` reports INDEX line numbers, and an
    // unstaged edit above a staged hunk shifts them out of step with the
    // buffer the bar is drawn against.
    let Some(all) = run_diff(parent, &abs, path, false) else {
        return (false, signs, Vec::new());
    };
    let unstaged = run_diff(parent, &abs, path, true).unwrap_or_default();

    let mut hunks = parse_diff_hunks_full(&all);
    let dirty = parse_diff_hunks_full(&unstaged);
    for h in &mut hunks {
        h.staged = !dirty.iter().any(|d| overlaps(h, d));
    }
    signs_from_hunks(&hunks, &mut signs);
    (true, signs, hunks)
}

/// What a gutter action does to one hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkAction {
    /// Add just this change to the index.
    Stage,
    /// Throw this change away, returning those lines to HEAD.
    Discard,
}

/// Apply `action` to the hunk covering `row` of `path`.
///
/// Built from the hunk's own `patch` — the bytes git produced go straight back
/// to `git apply`, so neither staging nor discarding has to re-derive what the
/// change was. Re-deriving it is how a one-hunk stage stages the wrong lines
/// when an earlier hunk in the same file has shifted them.
///
/// Returns the message to show, or an error.
pub fn apply_hunk(path: &str, row: usize, action: HunkAction) -> Result<String, String> {
    let abs = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    let root = crate::git_ops::find_git_root(Some(&abs))
        .ok_or_else(|| "Not in a git repository".to_string())?;
    let rel = abs
        .strip_prefix(&root)
        .map_err(|_| "File is outside the repository".to_string())?
        .to_string_lossy()
        .replace('\\', "/");

    let (_ok, _signs, hunks) = compute_gutter(path);
    let hunk = hunks
        .iter()
        .find(|h| h.contains(row))
        .ok_or_else(|| "No change on that line".to_string())?;

    if action == HunkAction::Stage && hunk.staged {
        return Err("Already staged".into());
    }

    // A minimal patch: the file header git needs to know what it is looking at,
    // then this hunk alone.
    let patch = format!(
        "diff --git a/{rel} b/{rel}\n--- a/{rel}\n+++ b/{rel}\n{}",
        hunk.patch
    );
    let args: &[&str] = match action {
        // `--cached` touches the index only, leaving the working tree alone —
        // which is the whole point of staging one hunk out of several.
        HunkAction::Stage => &["apply", "--cached", "--unidiff-zero", "-"],
        // `-R` against the working tree puts those lines back as HEAD has them.
        HunkAction::Discard => &["apply", "-R", "--unidiff-zero", "-"],
    };
    run_git_stdin(&root, args, &patch)?;
    Ok(match action {
        HunkAction::Stage => "Staged change".into(),
        HunkAction::Discard => "Discarded change".into(),
    })
}

/// `git` with a patch on stdin.
fn run_git_stdin(root: &Path, args: &[&str], stdin: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new("git")
        .args(args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    child
        .stdin
        .take()
        .ok_or_else(|| "git took no stdin".to_string())?
        .write_all(stdin.as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(err.lines().next().unwrap_or("git apply failed").to_string())
}

/// Do two hunks cover any of the same buffer lines?
///
/// Zero-length hunks (pure deletions) occupy the single line they are marked
/// against, so they compare as `[start, start]`.
fn overlaps(a: &GitHunk, b: &GitHunk) -> bool {
    let (a0, a1) = (a.start, a.end());
    let (b0, b1) = (b.start, b.end());
    a0 <= b1 && b0 <= a1
}

/// One `git diff -U0` against the index (`unstaged`) or against HEAD.
fn run_diff(parent: &Path, abs: &Path, path: &str, unstaged: bool) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("diff");
    if !unstaged {
        cmd.arg("HEAD");
    }
    cmd.args(["--no-color", "-U0", "--", abs.to_str().unwrap_or(path)]);
    let out = cmd.current_dir(parent).output().ok()?;
    if !out.status.success() && out.stdout.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[derive(Debug, Default, Clone)]
pub struct GitGutter {
    /// 0-based buffer line → sign. DERIVED from `hunks`.
    pub signs: HashMap<usize, GitSign>,
    /// The changes themselves, in buffer order. Everything the gutter can DO —
    /// draw one bar, highlight on hover, stage, discard, reveal what was
    /// removed — is per hunk; the sign map is just the per-line shadow of this.
    pub hunks: Vec<GitHunk>,
    pub path: String,
    pub available: bool,
}

impl GitGutter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.signs.clear();
        self.hunks.clear();
        self.path.clear();
        self.available = false;
    }

    /// The hunk covering `row`, if any.
    pub fn hunk_at(&self, row: usize) -> Option<&GitHunk> {
        self.hunks.iter().find(|h| h.contains(row))
    }

    /// Sync wrapper (hot paths use App's async channel + `compute_gutter`).
    pub fn refresh(&mut self, path: &str) {
        let (available, signs, hunks) = compute_gutter(path);
        self.path = path.to_string();
        self.available = available;
        self.signs = signs;
        self.hunks = hunks;
    }

    pub fn sign_at(&self, row: usize) -> Option<GitSign> {
        self.signs.get(&row).copied()
    }
}

/// Format blame for a narrow gutter: `ab  a1b2c3d`
pub fn format_blame_gutter(b: &BlameLine, width: usize) -> String {
    let s = format!(
        "{:<8} {}",
        b.author.chars().take(8).collect::<String>(),
        b.hash
    );
    s.chars().take(width).collect()
}

/// Parse a `-U0` unified diff into hunks.
///
/// `-U0` means no context, so every `@@` block IS one change and the `-` lines
/// under it are exactly what that change replaced. Keeping the removed text is
/// what makes "Show Change" possible without going back to git.
pub fn parse_diff_hunks_full(diff: &str) -> Vec<GitHunk> {
    let mut out: Vec<GitHunk> = Vec::new();
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("@@") {
            let mut old_count = 1i64;
            let mut new_start = 0i64;
            let mut new_count = 1i64;
            for p in rest.split_whitespace() {
                if let Some(spec) = p.strip_prefix('-') {
                    let (_s, c) = parse_hunk_spec(spec);
                    old_count = c;
                } else if let Some(spec) = p.strip_prefix('+') {
                    let (s, c) = parse_hunk_spec(spec);
                    new_start = s;
                    new_count = c;
                }
            }
            let n = new_count.max(0) as usize;
            let o = old_count.max(0) as usize;
            let kind = if o == 0 {
                GitSign::Added
            } else if n == 0 {
                GitSign::Deleted
            } else {
                GitSign::Modified
            };
            // A pure deletion has no line of its own; it is marked against the
            // line that now sits where the removed text was.
            let start = if n == 0 {
                (new_start.max(0) as usize).saturating_sub(0)
            } else {
                (new_start - 1).max(0) as usize
            };
            out.push(GitHunk {
                start,
                len: n,
                removed: Vec::new(),
                kind,
                patch: format!("{line}\n"),
                // Filled in by `compute_gutter`, which is the only caller with
                // both diffs to compare.
                staged: false,
            });
            continue;
        }
        // Body lines belong to the hunk most recently opened.
        //
        // The whole body is kept verbatim for `patch`; `removed` additionally
        // pulls out the old side, which is what "Show Change" reveals.
        if line.starts_with("---") || line.starts_with("+++") {
            continue;
        }
        let body = line.starts_with('+')
            || line.starts_with('-')
            || line.starts_with(' ')
            || line.starts_with('\\');
        if body {
            if let Some(h) = out.last_mut() {
                h.patch.push_str(line);
                h.patch.push('\n');
                if let Some(text) = line.strip_prefix('-') {
                    h.removed.push(text.to_string());
                }
            }
        }
    }
    out
}

/// The per-line signs a set of hunks produces.
///
/// Derived, so the bar the gutter draws and the sign a line reports cannot
/// disagree about where a change is.
pub fn signs_from_hunks(hunks: &[GitHunk], signs: &mut HashMap<usize, GitSign>) {
    for h in hunks {
        if h.len == 0 {
            signs.entry(h.start).or_insert(GitSign::Deleted);
            continue;
        }
        // A modification that removed more lines than it added still only
        // occupies the added ones; the surplus shows as a deletion marker on
        // the hunk's last line.
        for i in 0..h.len {
            let row = h.start + i;
            let sign = if h.kind == GitSign::Added || i >= h.removed.len() {
                GitSign::Added
            } else {
                GitSign::Modified
            };
            signs.insert(row, sign);
        }
        // A hunk that removed more lines than it added is a DELETION as far as
        // the reader is concerned, whatever else it also did: lines are gone
        // and the text left behind is the only trace. Its last row carries
        // that, overriding the Modified it just got.
        //
        // This was `or_insert`, which is the same statement with the opposite
        // effect — the row had already been claimed by the loop above, so the
        // entry was occupied and the deletion was dropped every single time.
        // Deleting three lines down to one reported a plain modification, and
        // the gutter drew it blue: "삭제했는데 파란색이야".
        if h.removed.len() > h.len {
            signs.insert(h.end(), GitSign::Deleted);
        }
    }
}

/// Parse unified diff hunks (`@@ -old,oc +new,nc @@`) into line signs.
pub fn parse_diff_hunks(diff: &str, signs: &mut HashMap<usize, GitSign>) {
    signs_from_hunks(&parse_diff_hunks_full(diff), signs);
}

fn parse_hunk_spec(spec: &str) -> (i64, i64) {
    // "10" or "10,3"
    if let Some((a, b)) = spec.split_once(',') {
        let s = a.parse().unwrap_or(0);
        let c = b.parse().unwrap_or(1);
        (s, c)
    } else {
        let s = spec.parse().unwrap_or(0);
        (s, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End to end against a real repository, because the staged/unstaged
    /// split is a claim about what two `git diff` invocations return and
    /// nothing but git can settle it.
    fn scratch_repo(name: &str) -> Option<std::path::PathBuf> {
        let dir = std::env::temp_dir().join(format!("suisei-gutter-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .ok()
                .filter(|o| o.status.success())
        };
        git(&["init", "-q", "."])?;
        git(&["config", "user.email", "t@example.invalid"])?;
        git(&["config", "user.name", "t"])?;
        std::fs::write(dir.join("f.rs"), "a\nb\nc\nd\ne\n").ok()?;
        git(&["add", "f.rs"])?;
        git(&["commit", "-qm", "init"])?;
        Some(dir)
    }

    #[test]
    fn an_unstaged_change_is_not_marked_staged() {
        let Some(dir) = scratch_repo("unstaged") else { return };
        std::fs::write(dir.join("f.rs"), "a\nb\nX\nY\nc\nd\ne\n").unwrap();
        let (ok, _signs, hunks) = compute_gutter(dir.join("f.rs").to_str().unwrap());
        assert!(ok, "git answered");
        assert_eq!(hunks.len(), 1, "one hunk: {hunks:?}");
        assert!(!hunks[0].staged, "not added to the index");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn staging_flips_the_hunk_without_moving_it() {
        let Some(dir) = scratch_repo("staged") else { return };
        std::fs::write(dir.join("f.rs"), "a\nb\nX\nY\nc\nd\ne\n").unwrap();
        let before = compute_gutter(dir.join("f.rs").to_str().unwrap()).2;

        assert!(
            Command::new("git")
                .args(["add", "f.rs"])
                .current_dir(&dir)
                .output()
                .is_ok_and(|o| o.status.success()),
            "staged it"
        );
        let after = compute_gutter(dir.join("f.rs").to_str().unwrap()).2;

        assert_eq!(after.len(), 1, "still one hunk: {after:?}");
        assert!(after[0].staged, "now staged");
        assert_eq!(
            (before[0].start, before[0].len),
            (after[0].start, after[0].len),
            "staging moves nothing — only the bar's fill changes"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shrinking_a_hunk_reports_a_deletion() {
        // Three lines collapsed to one. Git calls it a modification; the
        // person who did it calls it deleting two lines, and the gutter has to
        // agree with them — nothing else on screen shows the loss.
        let hunks = parse_diff_hunks_full("@@ -7,3 +7,1 @@\n-a\n-b\n-c\n+a\n");
        assert_eq!(hunks.len(), 1);
        assert_eq!((hunks[0].start, hunks[0].len), (6, 1));
        assert_eq!(hunks[0].removed.len(), 3);

        let mut signs = HashMap::new();
        signs_from_hunks(&hunks, &mut signs);
        assert_eq!(
            signs.get(&6),
            Some(&GitSign::Deleted),
            "the surviving row carries the loss"
        );
    }

    #[test]
    fn a_growing_hunk_is_not_a_deletion() {
        // One line expanded to three: nothing was lost, so nothing is red.
        let hunks = parse_diff_hunks_full("@@ -7,1 +7,3 @@\n-a\n+a\n+b\n+c\n");
        let mut signs = HashMap::new();
        signs_from_hunks(&hunks, &mut signs);
        assert_eq!(signs.get(&6), Some(&GitSign::Modified));
        assert_eq!(signs.get(&7), Some(&GitSign::Added));
        assert_eq!(signs.get(&8), Some(&GitSign::Added));
        assert!(!signs.values().any(|s| *s == GitSign::Deleted));
    }

    #[test]
    fn staging_one_hunk_leaves_the_other_alone() {
        let Some(dir) = scratch_repo("one-hunk") else { return };
        let f = dir.join("f.rs");
        // Two separate changes, far enough apart to be two hunks under -U0.
        std::fs::write(&f, "A\nb\nc\nd\nE\n").unwrap();
        let path = f.to_str().unwrap();

        let hunks = compute_gutter(path).2;
        assert_eq!(hunks.len(), 2, "two hunks: {hunks:?}");
        assert!(hunks.iter().all(|h| !h.staged));

        // Stage only the first.
        apply_hunk(path, hunks[0].start, HunkAction::Stage).unwrap();

        let after = compute_gutter(path).2;
        assert_eq!(after.len(), 2, "still two changes against HEAD");
        assert!(after[0].staged, "the one asked for is staged");
        assert!(!after[1].staged, "the other is untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discarding_one_hunk_restores_only_its_lines() {
        let Some(dir) = scratch_repo("discard-hunk") else { return };
        let f = dir.join("f.rs");
        std::fs::write(&f, "A\nb\nc\nd\nE\n").unwrap();
        let path = f.to_str().unwrap();

        let hunks = compute_gutter(path).2;
        assert_eq!(hunks.len(), 2);
        // Discard the LAST, so the first's line numbers are unaffected either
        // way and the test cannot pass by accident.
        apply_hunk(path, hunks[1].start, HunkAction::Discard).unwrap();

        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            "A\nb\nc\nd\ne\n",
            "only the discarded hunk went back to HEAD"
        );
        let after = compute_gutter(path).2;
        assert_eq!(after.len(), 1, "one change left: {after:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn staging_an_already_staged_hunk_says_so() {
        let Some(dir) = scratch_repo("restage") else { return };
        let f = dir.join("f.rs");
        std::fs::write(&f, "A\nb\nc\nd\ne\n").unwrap();
        let path = f.to_str().unwrap();
        let hunks = compute_gutter(path).2;
        apply_hunk(path, hunks[0].start, HunkAction::Stage).unwrap();
        assert!(
            apply_hunk(path, hunks[0].start, HunkAction::Stage).is_err(),
            "the menu should not offer to stage it twice"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hunks_carry_what_they_replaced() {
        // `-U0`: no context, so each `@@` block is exactly one change and the
        // `-` lines under it are what it replaced.
        let diff = "@@ -10,2 +10,1 @@\n-old one\n-old two\n+new\n";
        let hunks = parse_diff_hunks_full(diff);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].start, 9, "0-based");
        assert_eq!(hunks[0].len, 1);
        assert_eq!(hunks[0].kind, GitSign::Modified);
        assert_eq!(hunks[0].removed, vec!["old one", "old two"]);
    }

    #[test]
    fn a_pure_addition_replaced_nothing() {
        let hunks = parse_diff_hunks_full("@@ -5,0 +6,3 @@\n+a\n+b\n+c\n");
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].kind, GitSign::Added);
        assert_eq!(hunks[0].len, 3);
        assert!(hunks[0].removed.is_empty());
    }

    #[test]
    fn a_hunk_spans_its_whole_run() {
        // The reason the gutter drew a dotted column: three consecutive lines
        // are ONE change, not three.
        let hunks = parse_diff_hunks_full("@@ -1,3 +1,3 @@\n-a\n-b\n-c\n+x\n+y\n+z\n");
        assert_eq!(hunks.len(), 1);
        assert_eq!((hunks[0].start, hunks[0].len), (0, 3));
        assert_eq!(hunks[0].end(), 2);
        assert!(hunks[0].contains(0) && hunks[0].contains(2));
        assert!(!hunks[0].contains(3));
    }

    #[test]
    fn several_hunks_keep_their_own_removed_text() {
        let diff = "@@ -1,1 +1,1 @@\n-first\n+FIRST\n@@ -9,1 +9,1 @@\n-ninth\n+NINTH\n";
        let hunks = parse_diff_hunks_full(diff);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].removed, vec!["first"]);
        assert_eq!(hunks[1].removed, vec!["ninth"]);
        assert_eq!(hunks[1].start, 8);
    }

    #[test]
    fn the_diff_header_is_not_a_removed_line() {
        // `--- a/file` starts with '-' and is not content.
        let diff = "--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-real\n+new\n";
        let hunks = parse_diff_hunks_full(diff);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].removed, vec!["real"]);
    }

    #[test]
    fn parse_added_lines() {
        let mut m = HashMap::new();
        parse_diff_hunks("@@ -5,0 +6,2 @@\n+a\n+b\n", &mut m);
        assert_eq!(m.get(&5), Some(&GitSign::Added));
        assert_eq!(m.get(&6), Some(&GitSign::Added));
    }

    #[test]
    fn parse_modified_line() {
        let mut m = HashMap::new();
        parse_diff_hunks("@@ -10,1 +10,1 @@\n-old\n+new\n", &mut m);
        assert_eq!(m.get(&9), Some(&GitSign::Modified));
    }

    #[test]
    fn parse_deleted() {
        let mut m = HashMap::new();
        parse_diff_hunks("@@ -3,2 +3,0 @@\n-a\n-b\n", &mut m);
        assert!(m.values().any(|s| *s == GitSign::Deleted));
    }
}
