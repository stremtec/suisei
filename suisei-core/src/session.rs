//! Lightweight session restore: open files + cursor positions.

use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct SessionFile {
    pub path: String,
    pub row: usize,
    pub col: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Session {
    pub files: Vec<SessionFile>,
    pub active: usize,
}

fn session_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".xei").join("session")
}

/// Load session from `~/.xei/session`. Returns empty session if missing.
pub fn load() -> Session {
    let mut session = Session::default();
    let Ok(text) = fs::read_to_string(session_path()) else {
        return session;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(v) = line.strip_prefix("active=") {
            session.active = v.trim().parse().unwrap_or(0);
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
    session
}

/// Persist session to `~/.xei/session`.
pub fn save(session: &Session) {
    let path = session_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut out = String::from("# xei session — paths and cursor positions\n");
    out.push_str(&format!("active={}\n", session.active));
    for f in &session.files {
        if f.path.is_empty() {
            continue;
        }
        out.push_str(&format!("{}|{}|{}\n", f.path, f.row, f.col));
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
}
