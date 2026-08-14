//! Finding the programs Suisei shells out to.
//!
//! An app launched from Finder or the Dock inherits **Finder's** environment,
//! and its `PATH` is `/usr/bin:/bin:/usr/sbin:/sbin`. No Homebrew, no cargo, no
//! `~/.local/bin`. An app launched from a terminal inherits the user's shell
//! environment and has all of them.
//!
//! So `gh`, `rg` and `rust-analyzer` were installed or not installed depending
//! on how Suisei had been started that morning — with `Command::new("gh")`
//! reporting a plain "not found" either way. That is the whole of the reported
//! "gh를 제대로 찾지 못함": nothing was wrong with the GitHub integration except
//! that it could not see the binary.
//!
//! Two things are needed, and they are different:
//!
//! * **the program's own path**, so we can spawn it at all;
//! * **a `PATH` for the child**, because these programs shell out in turn —
//!   `gh` runs `git`, `cargo` runs `rustc`. Handing a child the environment
//!   that could not find its parent only moves the failure one level down.
//!
//! Deliberately NOT done by calling `std::env::set_var` at startup: the
//! environment is process-wide shared mutable state, which is why Rust 2024
//! made writing it `unsafe`, and a GUI process has AppKit's threads running
//! before any of our code does. Resolving per command is explicit, thread-safe,
//! and visible at the call site.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Where a developer's tools actually live, beyond whatever `PATH` we were
/// handed. Ordered by how likely they are to hold the newest copy.
///
/// `$HOME`-relative entries are expanded at lookup; a literal `~` is not a
/// path to anything.
const EXTRA_DIRS: &[&str] = &[
    "/opt/homebrew/bin",     // Homebrew, Apple silicon
    "/opt/homebrew/sbin",
    "/usr/local/bin",        // Homebrew, Intel — and most `make install`
    "/usr/local/sbin",
    "~/.cargo/bin",          // rustup: cargo, rust-analyzer
    "~/.local/bin",          // pipx, uv, and the XDG convention
    "~/.bun/bin",
    "/opt/local/bin",        // MacPorts
];

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn expand(dir: &str) -> Option<PathBuf> {
    match dir.strip_prefix("~/") {
        Some(rest) => home().map(|h| h.join(rest)),
        None => Some(PathBuf::from(dir)),
    }
}

/// Every directory to look in, in order: the inherited `PATH` first (a user who
/// launched from a terminal, or set one deliberately, gets exactly what they
/// asked for), then the standard locations that Finder's environment omits.
fn search_dirs() -> &'static [PathBuf] {
    static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    DIRS.get_or_init(|| {
        let mut out: Vec<PathBuf> = Vec::new();
        if let Some(path) = std::env::var_os("PATH") {
            out.extend(std::env::split_paths(&path));
        }
        for dir in EXTRA_DIRS {
            if let Some(p) = expand(dir) {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        out
    })
}

/// `PATH` for a child process — the search list, joined.
///
/// Handed to every spawned command so the tools those tools run are findable
/// too. Without it `gh auth login` finds no `git`, and `cargo` finds no
/// `rustc`, on exactly the launches where this module was needed at all.
pub fn child_path() -> &'static OsString {
    static JOINED: OnceLock<OsString> = OnceLock::new();
    JOINED.get_or_init(|| {
        std::env::join_paths(search_dirs()).unwrap_or_else(|_| {
            std::env::var_os("PATH").unwrap_or_else(|| OsString::from("/usr/bin:/bin"))
        })
    })
}

/// Absolute path of `program`, or `None` when it is genuinely not installed.
///
/// A name containing a separator is a path already and is returned as given —
/// the caller meant that file, not a search.
pub fn find(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let p = PathBuf::from(program);
        return p.exists().then_some(p);
    }
    search_dirs()
        .iter()
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Whether `program` can be run at all. The honest answer to "is `gh`
/// installed", as opposed to "did the launcher happen to give us a `PATH`
/// containing it".
pub fn is_available(program: &str) -> bool {
    find(program).is_some()
}

/// A [`Command`] for one of Suisei's tools: resolved to an absolute path when
/// we can find one, and carrying a `PATH` its child can work in.
///
/// Falls back to the bare name when the search comes up empty, so the failure
/// the caller sees is the operating system's own "no such file" rather than a
/// panic — and so a tool installed somewhere this module has never heard of
/// still works if the inherited `PATH` happens to know about it.
pub fn tool(program: &str) -> Command {
    let mut cmd = match find(program) {
        Some(abs) => Command::new(abs),
        None => Command::new(program),
    };
    cmd.env("PATH", child_path());
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The extras are appended, never substituted: a user who set `PATH`
    /// deliberately keeps their order and their overrides.
    #[test]
    fn the_inherited_path_comes_first() {
        let dirs = search_dirs();
        let inherited: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        assert!(
            dirs.len() >= inherited.len(),
            "the search list contains the inherited PATH"
        );
        for (a, b) in inherited.iter().zip(dirs.iter()) {
            assert_eq!(a, b, "in the order the user wrote it");
        }
    }

    /// The whole point: the directories Finder's environment leaves out are in
    /// the list whether or not the launcher mentioned them.
    #[test]
    fn homebrew_is_searched_even_when_path_omits_it() {
        let dirs = search_dirs();
        assert!(
            dirs.iter().any(|d| d.ends_with("homebrew/bin")),
            "Homebrew's bin is searched: {dirs:?}"
        );
        assert!(
            dirs.iter().any(|d| d == Path::new("/usr/local/bin")),
            "/usr/local/bin is searched"
        );
    }

    /// A path is not a name. Resolving `/bin/sh` must not go looking for a
    /// file called `/opt/homebrew/bin//bin/sh`.
    #[test]
    fn a_path_is_taken_as_given() {
        assert_eq!(find("/bin/sh"), Some(PathBuf::from("/bin/sh")));
        assert_eq!(find("/nonexistent/nope"), None);
    }

    /// Something every Unix has, found by name.
    #[test]
    fn a_system_tool_resolves_to_an_absolute_path() {
        let sh = find("sh").expect("sh is installed");
        assert!(sh.is_absolute(), "{sh:?}");
        assert!(is_executable(&sh));
    }

    /// A child gets a PATH containing the extras, so the tools our tools run
    /// are findable too.
    #[test]
    fn a_child_inherits_the_widened_path() {
        let joined = child_path().to_string_lossy().into_owned();
        assert!(joined.contains("/usr/local/bin"), "{joined}");
    }
}
