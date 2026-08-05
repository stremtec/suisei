//! Lightweight session restore: open files + cursor positions, and the
//! split layout (tree shape + per-pane viewports) when there was a split.

use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct SessionFile {
    pub path: String,
    pub row: usize,
    pub col: usize,
}

/// One pane's viewport, in visual order. `tab` indexes into `Session::files`.
#[derive(Debug, Clone, Default)]
pub struct SessionPane {
    pub tab: usize,
    pub scroll: usize,
    pub row: usize,
    pub col: usize,
}

/// Persisted split shape. `tree` is a token string the app layer converts
/// to/from its `Layout` (leaves are `T<tab>`; splits are
/// `S<C|R>:<w0,w1,...>:<child>;<child>;...`). None when the session had no
/// split.
#[derive(Debug, Clone, Default)]
pub struct SessionSplit {
    pub tree: String,
    pub focus_pane: usize,
    pub panes: Vec<SessionPane>,
}

#[derive(Debug, Clone, Default)]
pub struct Session {
    pub files: Vec<SessionFile>,
    pub active: usize,
    pub split: Option<SessionSplit>,
}

fn session_path() -> PathBuf {
    crate::fs_atomic::state_path("session")
}

/// Load session from `~/.suisei/session`. Returns empty session if missing.
pub fn load() -> Session {
    let mut session = Session::default();
    let Ok(text) = fs::read_to_string(session_path()) else {
        return session;
    };
    let mut split = SessionSplit::default();
    let mut have_split = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(v) = line.strip_prefix("active=") {
            session.active = v.trim().parse().unwrap_or(0);
            continue;
        }
        if let Some(v) = line.strip_prefix("split=") {
            split.tree = v.trim().to_string();
            have_split = !split.tree.is_empty();
            continue;
        }
        if let Some(v) = line.strip_prefix("focus_pane=") {
            split.focus_pane = v.trim().parse().unwrap_or(0);
            continue;
        }
        if let Some(v) = line.strip_prefix("pane=") {
            // pane=tab|scroll|row|col
            let parts: Vec<&str> = v.split('|').collect();
            if parts.len() >= 4 {
                split.panes.push(SessionPane {
                    tab: parts[0].parse().unwrap_or(0),
                    scroll: parts[1].parse().unwrap_or(0),
                    row: parts[2].parse().unwrap_or(0),
                    col: parts[3].parse().unwrap_or(0),
                });
            }
            continue;
        }
        // path|row|col
        let parts: Vec<&str> = line.split('|').collect();
        if parts.is_empty() || parts[0].is_empty() {
            continue;
        }
        let path = parts[0].to_string();
        let row = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let col = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
        // Skip files that no longer exist
        if !PathBuf::from(&path).exists() {
            continue;
        }
        session.files.push(SessionFile { path, row, col });
    }
    if session.active >= session.files.len() && !session.files.is_empty() {
        session.active = session.files.len() - 1;
    }
    if have_split && !split.panes.is_empty() {
        // Drop pane rows whose tab no longer loaded (its file vanished).
        split.panes.retain(|p| p.tab < session.files.len());
        if !split.panes.is_empty() {
            session.split = Some(split);
        }
    }
    session
}

/// Persist session to `~/.suisei/session`.
pub fn save(session: &Session) {
    let path = session_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut out = String::from("# suisei session — paths, cursors, split layout\n");
    out.push_str(&format!("active={}\n", session.active));
    for f in &session.files {
        if f.path.is_empty() {
            continue;
        }
        out.push_str(&format!("{}|{}|{}\n", f.path, f.row, f.col));
    }
    if let Some(split) = &session.split {
        out.push_str(&format!("split={}\n", split.tree));
        out.push_str(&format!("focus_pane={}\n", split.focus_pane));
        for p in &split.panes {
            out.push_str(&format!(
                "pane={}|{}|{}|{}\n",
                p.tab, p.scroll, p.row, p.col
            ));
        }
    }
    let _ = fs::write(path, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_format() {
        let s = Session {
            active: 1,
            files: vec![
                SessionFile {
                    path: "/tmp/a.rs".into(),
                    row: 2,
                    col: 3,
                },
                SessionFile {
                    path: "/tmp/b.rs".into(),
                    row: 0,
                    col: 0,
                },
            ],
            split: None,
        };
        // Manual serialize/deserialize of format without writing home
        let mut text = format!("active={}\n", s.active);
        for f in &s.files {
            text.push_str(&format!("{}|{}|{}\n", f.path, f.row, f.col));
        }
        let mut parsed = Session::default();
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("active=") {
                parsed.active = v.parse().unwrap();
            } else {
                let parts: Vec<&str> = line.split('|').collect();
                parsed.files.push(SessionFile {
                    path: parts[0].into(),
                    row: parts[1].parse().unwrap(),
                    col: parts[2].parse().unwrap(),
                });
            }
        }
        assert_eq!(parsed.active, 1);
        assert_eq!(parsed.files.len(), 2);
        assert_eq!(parsed.files[0].row, 2);
    }

    /// Split layout lines round-trip: tree tokens, focus, per-pane viewports.
    #[test]
    fn split_roundtrip_format() {
        let text = "active=0\n/tmp/a.rs|0|0\n/tmp/b.rs|5|2\nsplit=SC:0.500,0.500:T0;T1\nfocus_pane=1\npane=0|0|0|0\npane=1|120|5|2\n";
        let mut parsed = Session::default();
        let mut split = SessionSplit::default();
        let mut have = false;
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("split=") {
                split.tree = v.to_string();
                have = true;
            } else if let Some(v) = line.strip_prefix("focus_pane=") {
                split.focus_pane = v.parse().unwrap();
            } else if let Some(v) = line.strip_prefix("pane=") {
                let p: Vec<&str> = v.split('|').collect();
                split.panes.push(SessionPane {
                    tab: p[0].parse().unwrap(),
                    scroll: p[1].parse().unwrap(),
                    row: p[2].parse().unwrap(),
                    col: p[3].parse().unwrap(),
                });
            }
        }
        assert!(have);
        parsed.split = Some(split);
        let s = parsed.split.unwrap();
        assert_eq!(s.tree, "SC:0.500,0.500:T0;T1");
        assert_eq!(s.focus_pane, 1);
        assert_eq!(s.panes.len(), 2);
        assert_eq!(s.panes[1].scroll, 120);
        assert_eq!(s.panes[1].row, 5);
    }
}
