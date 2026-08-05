//! Project-wide text search (ripgrep preferred, walk fallback).

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub path: PathBuf,
    pub row: usize, // 0-based
    pub col: usize, // 0-based char (best-effort)
    pub line: String,
}

#[derive(Debug, Clone)]
pub struct WorkspaceSearch {
    pub open: bool,
    pub query: String,
    pub replace: String,
    /// When true, focus is on the replace field.
    pub replace_focus: bool,
    pub hits: Vec<SearchHit>,
    pub selected: usize,
    pub scroll: usize,
    pub status: String,
    pub root: PathBuf,
    /// Dirty flag: re-run search on next idle when query changes.
    pub needs_search: bool,
}

impl Default for WorkspaceSearch {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            replace: String::new(),
            replace_focus: false,
            hits: Vec::new(),
            selected: 0,
            scroll: 0,
            status: String::new(),
            root: PathBuf::from("."),
            needs_search: false,
        }
    }
}

impl WorkspaceSearch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open_at(&mut self, root: PathBuf) {
        self.open = true;
        self.root = root;
        self.query.clear();
        self.replace.clear();
        self.replace_focus = false;
        self.hits.clear();
        self.selected = 0;
        self.scroll = 0;
        self.status = "Type to search · Tab replace · Enter open · r replace one · R all".into();
        self.needs_search = false;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.hits.clear();
        self.query.clear();
        self.replace.clear();
        self.needs_search = false;
    }

    pub fn push_char(&mut self, c: char) {
        if self.replace_focus {
            self.replace.push(c);
        } else {
            self.query.push(c);
            self.needs_search = true;
        }
    }

    pub fn pop_char(&mut self) {
        if self.replace_focus {
            self.replace.pop();
        } else {
            self.query.pop();
            self.needs_search = true;
        }
    }

    pub fn toggle_replace_focus(&mut self) {
        self.replace_focus = !self.replace_focus;
    }

    pub fn move_sel(&mut self, delta: isize) {
        if self.hits.is_empty() {
            self.selected = 0;
            return;
        }
        let n = self.hits.len() as isize;
        let cur = self.selected as isize + delta;
        self.selected = (((cur % n) + n) % n) as usize;
        // keep in view roughly
        if self.selected < self.scroll {
            self.scroll = self.selected;
        }
    }

    pub fn selected_hit(&self) -> Option<&SearchHit> {
        self.hits.get(self.selected)
    }

    /// Run search if query non-empty. Prefer `rg`, fall back to walk.
    pub fn run_search(&mut self) {
        self.needs_search = false;
        self.hits.clear();
        self.selected = 0;
        self.scroll = 0;
        let q = self.query.trim();
        if q.is_empty() {
            self.status = "Type a pattern…".into();
            return;
        }
        self.hits = search_project(&self.root, q, 500);
        self.status = if self.hits.is_empty() {
            format!("No matches for `{q}`")
        } else {
            format!("{} match(es) in {}", self.hits.len(), self.root.display())
        };
    }
}

/// Search with ripgrep when available; otherwise recursive walk + line scan.
pub fn search_project(root: &Path, pattern: &str, max: usize) -> Vec<SearchHit> {
    if let Some(hits) = search_with_rg(root, pattern, max) {
        return hits;
    }
    search_walk(root, pattern, max)
}

fn search_with_rg(root: &Path, pattern: &str, max: usize) -> Option<Vec<SearchHit>> {
    let output = Command::new("rg")
        .args([
            "--json",
            "--max-count",
            "50",
            "-m",
            &max.to_string(),
            "--hidden",
            "--glob",
            "!.git",
            "--glob",
            "!target",
            "--glob",
            "!node_modules",
            "--glob",
            "!dist",
            "--glob",
            "!build",
            "-n",
            "--",
            pattern,
        ])
        .current_dir(root)
        .output()
        .ok()?;
    // rg returns 1 when no matches
    if !output.status.success() && output.stdout.is_empty() {
        if output.status.code() == Some(1) {
            return Some(Vec::new());
        }
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut hits = Vec::new();
    for line in text.lines() {
        if !line.contains("\"type\":\"match\"") {
            continue;
        }
        // crude JSON field extract
        let path = extract_json_str(line, "\"path\":{\"text\":\"")
            .or_else(|| extract_json_str(line, "\"path\":\""));
        let row = extract_json_num(line, "\"line_number\":");
        let text_line = extract_json_str(line, "\"lines\":{\"text\":\"")
            .or_else(|| extract_json_str(line, "\"text\":\""));
        let col = extract_json_num(line, "\"start\":").unwrap_or(0);
        if let (Some(p), Some(r), Some(tl)) = (path, row, text_line) {
            let line_clean = tl.trim_end_matches(['\r', '\n']).to_string();
            hits.push(SearchHit {
                path: root.join(p),
                row: r.saturating_sub(1),
                col,
                line: line_clean,
            });
            if hits.len() >= max {
                break;
            }
        }
    }
    Some(hits)
}

fn extract_json_str(s: &str, key: &str) -> Option<String> {
    let i = s.find(key)? + key.len();
    let rest = &s[i..];
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                match n {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    'u' => {
                        // skip \uXXXX
                        let hex: String = chars.by_ref().take(4).collect();
                        if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                            if let Some(ch) = char::from_u32(cp) {
                                out.push(ch);
                            }
                        }
                    }
                    other => out.push(other),
                }
            }
            continue;
        }
        if c == '"' {
            break;
        }
        out.push(c);
    }
    if out.is_empty() && !rest.starts_with('"') {
        // already consumed start
    }
    Some(out)
}

fn extract_json_num(s: &str, key: &str) -> Option<usize> {
    let i = s.find(key)? + key.len();
    let rest = &s[i..];
    let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    num.parse().ok()
}

fn search_walk(root: &Path, pattern: &str, max: usize) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let skip = ["target", "node_modules", ".git", "dist", "build", ".xei"];
    let pat_lower = pattern.to_lowercase();
    let case_sensitive = pattern.chars().any(|c| c.is_uppercase());

    while let Some(dir) = stack.pop() {
        if hits.len() >= max {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            if hits.len() >= max {
                break;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') && name != ".env" {
                continue;
            }
            if path.is_dir() {
                if skip.iter().any(|s| *s == name) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            // skip binary-ish
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let e = ext.to_lowercase();
                if matches!(
                    e.as_str(),
                    "png"
                        | "jpg"
                        | "jpeg"
                        | "gif"
                        | "webp"
                        | "pdf"
                        | "zip"
                        | "o"
                        | "a"
                        | "so"
                        | "dylib"
                        | "exe"
                        | "wasm"
                        | "bin"
                ) {
                    continue;
                }
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            for (i, line) in content.lines().enumerate() {
                let found = if case_sensitive {
                    line.find(pattern)
                } else {
                    line.to_lowercase().find(&pat_lower)
                };
                if let Some(col) = found {
                    hits.push(SearchHit {
                        path: path.clone(),
                        row: i,
                        col,
                        line: line.to_string(),
                    });
                    if hits.len() >= max {
                        break;
                    }
                }
            }
        }
    }
    hits
}

/// Replace first occurrence of `query` on a specific line of a file. Returns true if changed.
pub fn replace_in_file(
    path: &Path,
    row: usize,
    query: &str,
    replace: &str,
) -> Result<bool, String> {
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    // preserve trailing newline presence
    let trailing = content.ends_with('\n');
    if row >= lines.len() {
        return Err("line out of range".into());
    }
    if !lines[row].contains(query) {
        return Ok(false);
    }
    lines[row] = lines[row].replacen(query, replace, 1);
    let mut out = lines.join("\n");
    if trailing {
        out.push('\n');
    }
    crate::fs_atomic::atomic_write_file(path, out).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Replace every occurrence of `query` in a file. Returns number of replacements.
pub fn replace_all_in_file(path: &Path, query: &str, replace: &str) -> Result<usize, String> {
    if query.is_empty() {
        return Ok(0);
    }
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let count = content.matches(query).count();
    if count == 0 {
        return Ok(0);
    }
    let out = content.replace(query, replace);
    crate::fs_atomic::atomic_write_file(path, out).map_err(|e| e.to_string())?;
    Ok(count)
}

/// Replace first match of `query` on each hit's line (one replace per hit path+row).
/// Returns total successful replacements.
pub fn replace_hits(hits: &[SearchHit], query: &str, replace: &str) -> Result<usize, String> {
    let mut n = 0usize;
    for h in hits {
        if replace_in_file(Path::new(&h.path), h.row, query, replace)? {
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn walk_finds_pattern() {
        let dir = std::env::temp_dir().join(format!("xei_ws_search_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("hello.txt");
        let mut file = std::fs::File::create(&f).unwrap();
        writeln!(file, "alpha").unwrap();
        writeln!(file, "findme now").unwrap();
        writeln!(file, "beta").unwrap();
        let hits = search_walk(&dir, "findme", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].row, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_one_line() {
        let dir = std::env::temp_dir().join(format!("xei_ws_repl_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.txt");
        std::fs::write(&f, "one two one\n").unwrap();
        assert!(replace_in_file(&f, 0, "one", "ONE").unwrap());
        let s = std::fs::read_to_string(&f).unwrap();
        assert_eq!(s, "ONE two one\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_all_in_file_changes_every_match() {
        let dir = std::env::temp_dir().join(format!(
            "xei_ws_repl_all_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("b.txt");
        std::fs::write(&f, "one two one three one\n").unwrap();
        let n = replace_all_in_file(&f, "one", "ONE").unwrap();
        assert_eq!(n, 3);
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            "ONE two ONE three ONE\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_hits_uses_real_replace_in_file() {
        let dir = std::env::temp_dir().join(format!(
            "xei_ws_hits_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("c.txt");
        std::fs::write(&f, "alpha\nfindme now\nbeta findme\n").unwrap();
        let hits = search_walk(&dir, "findme", 10);
        assert_eq!(hits.len(), 2);
        let n = replace_hits(&hits, "findme", "FOUND").unwrap();
        assert_eq!(n, 2);
        let s = std::fs::read_to_string(&f).unwrap();
        assert!(s.contains("FOUND now"));
        assert!(s.contains("beta FOUND"));
        assert!(!s.contains("findme"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
