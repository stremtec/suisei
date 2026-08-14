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

/// One restored tab. A document, or a shell.
///
/// Terminals were skipped entirely — `save_session` only kept tabs with a
/// filename — so a window full of shells came back empty, and a split holding
/// one could not be saved at all, because every pane had to resolve to a saved
/// file. They occupy a slot in the same ordered list now, which is what keeps
/// the split's pane indices meaning the same thing on both sides.
///
/// A shell restores as a TAB AT A DIRECTORY, not as a living process: the
/// program that was running is gone with the machine's process table, and
/// pretending otherwise would restore a prompt that lies about its history.
#[derive(Debug, Clone)]
pub enum SessionItem {
    File(SessionFile),
    Terminal { cwd: String },
}

impl SessionItem {
    /// The file this item is, if it is one — the split restore only points at
    /// documents it can reopen.
    pub fn file(&self) -> Option<&SessionFile> {
        match self {
            SessionItem::File(f) => Some(f),
            SessionItem::Terminal { .. } => None,
        }
    }
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
    /// Tabs in strip order — documents and shells together, because the
    /// split's `pane.tab` indexes into this and a list missing its terminals
    /// would point every pane after one at the wrong tab.
    pub items: Vec<SessionItem>,
    pub active: usize,
    pub split: Option<SessionSplit>,
}

fn session_path() -> PathBuf {
    crate::fs_atomic::state_path("session")
}

/// Load session from `~/.suisei/session`. Returns empty session if missing.
pub fn load() -> Session {
    let Ok(text) = fs::read_to_string(session_path()) else {
        return Session::default();
    };
    parse(&text)
}

/// The format, as a pure function.
///
/// Split out from `load` so the tests can exercise THIS rather than their own
/// copy of the format — which is what they used to do, and why a parser that
/// had never seen a `term=` line could still pass them.
pub fn parse(text: &str) -> Session {
    let mut session = Session::default();
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
        if let Some(v) = line.strip_prefix("term=") {
            // A shell, in order with the documents around it.
            let cwd = v.trim().to_string();
            // A directory that has since gone takes the tab with it, exactly
            // as a deleted file does below.
            if !cwd.is_empty() && PathBuf::from(&cwd).is_dir() {
                session.items.push(SessionItem::Terminal { cwd });
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
        session
            .items
            .push(SessionItem::File(SessionFile { path, row, col }));
    }
    if session.active >= session.items.len() && !session.items.is_empty() {
        session.active = session.items.len() - 1;
    }
    if have_split && !split.panes.is_empty() {
        // Drop pane rows whose tab no longer loaded (its file vanished).
        split.panes.retain(|p| p.tab < session.items.len());
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
    let _ = fs::write(path, render(session));
}

/// The other half of the format. Same reason as `parse`.
pub fn render(session: &Session) -> String {
    let mut out = String::from("# suisei session — paths, cursors, split layout\n");
    out.push_str(&format!("active={}\n", session.active));
    for item in &session.items {
        match item {
            SessionItem::File(f) => {
                if f.path.is_empty() {
                    continue;
                }
                out.push_str(&format!("{}|{}|{}\n", f.path, f.row, f.col));
            }
            SessionItem::Terminal { cwd } => {
                out.push_str(&format!("term={cwd}\n"));
            }
        }
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
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp directory that really exists, because `parse` drops entries whose
    /// path is gone — and testing the format without that filter would be
    /// testing a different function.
    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("suisei_session_{tag}_{}", std::process::id()));
        let _ = fs::create_dir_all(&d);
        d
    }

    #[test]
    fn roundtrip_through_the_real_format() {
        let dir = scratch("rt");
        let a = dir.join("a.rs");
        let b = dir.join("b.rs");
        fs::write(&a, "").unwrap();
        fs::write(&b, "").unwrap();

        let s = Session {
            active: 1,
            items: vec![
                SessionItem::File(SessionFile {
                    path: a.display().to_string(),
                    row: 2,
                    col: 3,
                }),
                SessionItem::File(SessionFile {
                    path: b.display().to_string(),
                    row: 0,
                    col: 0,
                }),
            ],
            split: None,
        };
        // `render`/`parse`, not a hand-written copy of the format. The tests
        // this replaces reimplemented both sides, so they agreed with each
        // other and never with the parser.
        let back = parse(&render(&s));
        assert_eq!(back.active, 1);
        assert_eq!(back.items.len(), 2);
        assert_eq!(back.items[0].file().unwrap().row, 2);
        let _ = fs::remove_dir_all(dir);
    }

    /// A shell keeps its slot, and its directory.
    #[test]
    fn a_terminal_survives_the_round_trip() {
        let dir = scratch("term");
        let f = dir.join("x.rs");
        fs::write(&f, "").unwrap();

        let s = Session {
            active: 0,
            items: vec![
                SessionItem::File(SessionFile {
                    path: f.display().to_string(),
                    row: 0,
                    col: 0,
                }),
                SessionItem::Terminal {
                    cwd: dir.display().to_string(),
                },
            ],
            split: None,
        };
        let back = parse(&render(&s));
        assert_eq!(back.items.len(), 2, "the shell was dropped: {:?}", back.items);
        match &back.items[1] {
            SessionItem::Terminal { cwd } => {
                assert_eq!(cwd, &dir.display().to_string())
            }
            other => panic!("expected a terminal, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    /// THE reason shells occupy a slot rather than being skipped.
    ///
    /// `pane.tab` indexes this list. A terminal that is saved but not listed
    /// shifts every document after it, and each pane then restores the wrong
    /// tab — silently, because the indices are all still in range.
    #[test]
    fn a_terminal_does_not_shift_the_documents_after_it() {
        let dir = scratch("idx");
        let a = dir.join("a.rs");
        let b = dir.join("b.rs");
        fs::write(&a, "").unwrap();
        fs::write(&b, "").unwrap();

        let s = Session {
            active: 0,
            items: vec![
                SessionItem::File(SessionFile {
                    path: a.display().to_string(),
                    row: 0,
                    col: 0,
                }),
                SessionItem::Terminal {
                    cwd: dir.display().to_string(),
                },
                SessionItem::File(SessionFile {
                    path: b.display().to_string(),
                    row: 7,
                    col: 1,
                }),
            ],
            split: None,
        };
        let back = parse(&render(&s));
        assert_eq!(back.items.len(), 3);
        assert!(back.items[1].file().is_none(), "slot 1 is the shell");
        let last = back.items[2].file().expect("slot 2 is still b.rs");
        assert_eq!(last.row, 7, "the document after the shell moved");
        assert!(last.path.ends_with("b.rs"));
        let _ = fs::remove_dir_all(dir);
    }

    /// A directory that has gone takes its tab with it, exactly as a deleted
    /// file does — otherwise restore spawns a shell that cannot chdir.
    #[test]
    fn a_terminal_whose_directory_vanished_is_dropped() {
        let gone = std::env::temp_dir().join("suisei_session_absent_dir_xyz");
        let _ = fs::remove_dir_all(&gone);
        let text = format!("active=0\nterm={}\n", gone.display());
        assert!(parse(&text).items.is_empty());
    }

    /// Split layout lines round-trip: tree tokens, focus, per-pane viewports.
    #[test]
    fn split_roundtrip_format() {
        let dir = scratch("split");
        let a = dir.join("a.rs");
        let b = dir.join("b.rs");
        fs::write(&a, "").unwrap();
        fs::write(&b, "").unwrap();
        let text = format!(
            "active=0\n{}|0|0\n{}|5|2\nsplit=SC:0.500,0.500:T0;T1\nfocus_pane=1\npane=0|0|0|0\npane=1|120|5|2\n",
            a.display(),
            b.display()
        );
        let parsed = parse(&text);
        let split = parsed.split.expect("split was parsed");
        assert_eq!(split.tree, "SC:0.500,0.500:T0;T1");
        assert_eq!(split.focus_pane, 1);
        assert_eq!(split.panes.len(), 2);
        assert_eq!(split.panes[1].scroll, 120);
        let _ = fs::remove_dir_all(dir);
    }
}
