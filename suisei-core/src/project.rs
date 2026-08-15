//! `project.suiseiprj` — the file that says "this folder is a project".
//!
//! # What it is for
//!
//! Suisei's idea of a project was, until now, whichever folder happened to be
//! open. That is enough to index and to resolve a root, and not enough for two
//! things the user asked for:
//!
//! * a **Recents** list where projects are projects. Without a marker a folder
//!   and a file are the same kind of entry, so the three folders you actually
//!   work in sink under the twelve files you opened last;
//! * a **project identity** that survives being moved, renamed, cloned to
//!   another machine, or opened by somebody else. `project_id` is that.
//!
//! # What it must NOT contain
//!
//! No members, no roles, no tokens, no e-mail addresses. A file in the
//! repository is editable by anyone who can edit the repository, so a member
//! list in it is a permission you can grant yourself with a text editor — and
//! one that a merge conflict can scramble. Membership belongs to a server that
//! can refuse. This file carries the identifier that server knows the project
//! by, and nothing else worth stealing.
//!
//! The identifier is not a secret either. It names a project; it does not
//! authorise anything.

use std::io;
use std::path::{Path, PathBuf};

/// The marker's filename. Committed to the repository on purpose — a project
/// is a project for everyone who clones it.
pub const MARKER: &str = "project.suiseiprj";

/// What the marker holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// Format version, so a future field can be added without guessing.
    pub schema: u32,
    /// Stable identity, assigned once. Survives a rename or a move, which the
    /// folder's own name does not.
    pub project_id: String,
    /// Display name at creation. Advisory: the folder's name wins if they
    /// disagree, because that is what the user sees in Finder.
    pub name: String,
}

impl Project {
    fn to_json(&self) -> String {
        // Written by hand rather than through a serializer so the file on disk
        // is stable, readable and diff-friendly — it lives in a repository and
        // a human will open it.
        format!(
            "{{\n  \"schema\": {},\n  \"project_id\": {},\n  \"name\": {}\n}}\n",
            self.schema,
            json_string(&self.project_id),
            json_string(&self.name),
        )
    }
}

fn json_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Whether this exact directory is a project root.
pub fn is_project(dir: &Path) -> bool {
    dir.join(MARKER).is_file()
}

/// The project root at or above `from`, if any.
///
/// Walks up, so a file deep inside a project resolves to the project rather
/// than to its own folder. The NEAREST marker wins: nesting is refused when a
/// project is created, but a clone can arrive already nested and the innermost
/// answer is the least surprising one.
pub fn find_root(from: &Path) -> Option<PathBuf> {
    let mut dir = if from.is_dir() {
        Some(from.to_path_buf())
    } else {
        from.parent().map(Path::to_path_buf)
    };
    while let Some(d) = dir {
        if is_project(&d) {
            return Some(d);
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    None
}

/// Read the marker, if this directory has one.
///
/// Tolerant: a marker whose JSON is damaged still identifies the folder as a
/// project, with whatever fields survived. Refusing to open a project because
/// somebody hand-edited a comment into its marker would be the worse failure.
pub fn read(dir: &Path) -> Option<Project> {
    let text = std::fs::read_to_string(dir.join(MARKER)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
    Some(Project {
        schema: v.get("schema").and_then(|x| x.as_u64()).unwrap_or(1) as u32,
        project_id: v
            .get("project_id")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        name: v
            .get("name")
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| dir_name(dir)),
    })
}

/// Mark `dir` as a project, or return the marker it already has.
///
/// Idempotent on purpose. "Set project master directory" and "New Project" both
/// land here, and so does opening a folder that is already a project — a second
/// call must not mint a new identity for a project that has one, or the same
/// folder would be two projects to anything that remembers the id.
pub fn ensure(dir: &Path) -> io::Result<Project> {
    if let Some(existing) = read(dir) {
        return Ok(existing);
    }
    let project = Project {
        schema: 1,
        project_id: new_id(dir),
        name: dir_name(dir),
    };
    // Written whole rather than appended: this is a small file and a partial
    // one is worse than none.
    std::fs::write(dir.join(MARKER), project.to_json())?;
    Ok(project)
}

fn dir_name(dir: &Path) -> String {
    dir.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Project")
        .to_string()
}

/// `prj_` + 26 lowercase base32 characters, from the clock and the path.
///
/// Not a UUID crate, because the requirement is "two projects on this machine
/// do not collide", not "no two projects in the universe do" — and the id is a
/// name, not a capability, so guessing one buys nothing. The path is mixed in
/// so two projects created in the same millisecond still differ.
fn new_id(dir: &Path) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in dir.as_os_str().as_encoded_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let mut out = String::from("prj_");
    for word in [nanos, h] {
        let mut v = word;
        for _ in 0..13 {
            out.push(ALPHABET[(v & 0x1f) as usize] as char);
            v >>= 5;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("suisei_prj_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn ensure_writes_a_marker_and_reads_back() {
        let d = tmp("basic");
        assert!(!is_project(&d));
        let p = ensure(&d).unwrap();
        assert!(is_project(&d));
        assert!(p.project_id.starts_with("prj_"));
        assert_eq!(read(&d).unwrap(), p);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The identity is assigned ONCE. Both "New Project" and "Set project
    /// master directory" call this, and so does opening a folder that already
    /// is one — minting a new id on the second call would make one folder two
    /// projects to anything that remembers the first.
    #[test]
    fn ensure_is_idempotent() {
        let d = tmp("idem");
        let a = ensure(&d).unwrap();
        let b = ensure(&d).unwrap();
        assert_eq!(a.project_id, b.project_id);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A file deep inside resolves to the project, not to its own folder.
    #[test]
    fn find_root_walks_up_from_a_file() {
        let d = tmp("walk");
        let deep = d.join("src").join("a");
        std::fs::create_dir_all(&deep).unwrap();
        let file = deep.join("main.rs");
        std::fs::write(&file, "fn main() {}").unwrap();

        assert_eq!(find_root(&file), None, "no marker, no project");
        ensure(&d).unwrap();
        assert_eq!(find_root(&file).unwrap(), d);
        assert_eq!(find_root(&deep).unwrap(), d);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The nearest marker wins. Creating a nested project is refused by the
    /// face, but a clone can arrive already nested and the innermost answer is
    /// the least surprising one.
    #[test]
    fn the_nearest_marker_wins() {
        let d = tmp("nested");
        let inner = d.join("sub");
        std::fs::create_dir_all(&inner).unwrap();
        ensure(&d).unwrap();
        ensure(&inner).unwrap();
        assert_eq!(find_root(&inner.join("x.rs")).unwrap(), inner);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A hand-damaged marker still identifies a project. Refusing to open one
    /// because somebody typed into its marker is the worse failure.
    #[test]
    fn a_damaged_marker_still_names_a_project() {
        let d = tmp("damaged");
        std::fs::write(d.join(MARKER), "{ this is not json").unwrap();
        assert!(is_project(&d));
        let p = read(&d).unwrap();
        assert_eq!(p.name, d.file_name().unwrap().to_str().unwrap());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Ids differ for projects created back to back, which is the only
    /// collision that can actually happen on one machine.
    #[test]
    fn ids_differ_between_projects() {
        let a = tmp("id_a");
        let b = tmp("id_b");
        assert_ne!(ensure(&a).unwrap().project_id, ensure(&b).unwrap().project_id);
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    /// What must never be in the file: anything that grants something. A member
    /// list here is a permission you can award yourself with a text editor.
    #[test]
    fn the_marker_carries_no_authority() {
        let d = tmp("authority");
        ensure(&d).unwrap();
        let text = std::fs::read_to_string(d.join(MARKER)).unwrap();
        for forbidden in ["member", "role", "token", "admin", "owner", "email"] {
            assert!(
                !text.to_lowercase().contains(forbidden),
                "marker must not carry {forbidden}: {text}"
            );
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
