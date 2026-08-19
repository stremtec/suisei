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
    // ── Node, which is where the JavaScript and TypeScript servers live ──
    //
    // `typescript-language-server` is installed with `npm i -g` — the line
    // this editor prints when it is missing — and on a developer's Mac Node
    // itself usually comes from a version manager, not from Homebrew. None of
    // those put their `bin` anywhere above, so Suisei concluded the server was
    // not installed on machines that had it, and hover and ⌘-click did nothing
    // and said nothing. **This is the exact bug `exec` exists to end**,
    // recurring for a different toolchain.
    "~/.volta/bin",          // Volta
    "~/Library/pnpm",        // pnpm's global bin on macOS
    "~/.npm-global/bin",     // the documented custom `npm prefix`
    "~/.asdf/shims",         // asdf
    "~/.nodenv/shims",       // nodenv
    // `go install` — which is the line printed for Delve — writes to
    // `$GOPATH/bin`, and the default GOPATH is `~/go`. Nothing else puts a
    // binary there, and it was on no list, so the Go debug adapter was
    // permanently "Not Installed" on machines that had just installed it.
    "~/go/bin",
];

/// Node version managers that keep one directory per installed version.
///
/// nvm and fnm have no fixed `bin` — the path carries the version
/// (`~/.nvm/versions/node/v22.3.0/bin`), so the only way to find a globally
/// installed tool is to look at what is there. Every version is searched,
/// newest first by name: a tool installed under an old Node still runs, and
/// preferring the newest matches what the shell would have picked.
const VERSIONED_NODE_DIRS: &[(&str, &str)] = &[
    ("~/.nvm/versions/node", "bin"),
    ("~/.fnm/node-versions", "installation/bin"),
    ("~/Library/Application Support/fnm/node-versions", "installation/bin"),
];

/// Every per-version `bin` under the managers above, newest name first.
fn node_version_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for (root, tail) in VERSIONED_NODE_DIRS {
        let Some(base) = expand(root) else { continue };
        let Ok(entries) = std::fs::read_dir(&base) else {
            continue;
        };
        let mut versions: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        // By NAME, descending. Not semver-aware — `v9` sorts above `v10` —
        // which is wrong only in the order two installed versions are tried,
        // and both are searched either way.
        versions.sort();
        versions.reverse();
        out.extend(versions.into_iter().map(|v| v.join(tail)).filter(|p| p.is_dir()));
    }
    out
}

/// Apple's own toolchain, which is on nobody's `PATH`.
///
/// `lldb-dap` — the debug adapter for Rust, C and C++ — ships inside Xcode and
/// inside the Command Line Tools, and **neither directory is on `PATH` even in
/// a terminal**. Developers reach these through `xcrun`, so a search that only
/// walks `PATH` concludes the debugger is not installed on a Mac that has had
/// it all along. Measured on this machine: `which lldb-dap` finds nothing while
/// both `/Applications/Xcode.app/Contents/Developer/usr/bin/lldb-dap` and
/// `/Library/Developer/CommandLineTools/usr/bin/lldb-dap` exist.
///
/// `xcode-select -p` is asked rather than assumed, because it is the thing that
/// decides WHICH toolchain is active — a machine with both installed can have
/// either selected, and hard-coding the Xcode path would silently use the one
/// the user switched away from. The literal Command Line Tools directory is
/// still appended: `xcode-select` can point at an Xcode that has since been
/// deleted, and then the CLT copy is the one that works.
fn developer_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    // Absolute path deliberately: this runs while we are still working out
    // where anything is, so it cannot depend on the search it is feeding.
    if let Ok(o) = Command::new("/usr/bin/xcode-select").arg("-p").output() {
        if o.status.success() {
            let selected = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !selected.is_empty() {
                out.push(PathBuf::from(&selected).join("usr/bin"));
            }
        }
    }
    let clt = PathBuf::from("/Library/Developer/CommandLineTools/usr/bin");
    if !out.contains(&clt) {
        out.push(clt);
    }
    out
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn expand(dir: &str) -> Option<PathBuf> {
    match dir.strip_prefix("~/") {
        Some(rest) => home().map(|h| h.join(rest)),
        None => Some(PathBuf::from(dir)),
    }
}

/// The `PATH` the user's own login shell would hand them.
///
/// **Enumerating version managers is a losing game.** The list above covers
/// nvm, fnm, Volta, pnpm, asdf and nodenv, and the machine this was written on
/// keeps Node in `~/.hermes/node/bin` — on no list anywhere, and there will
/// always be another one. The shell already knows where everything is, because
/// the user's own profile is what put it there. So ask it, once.
///
/// `-l -i` because a PATH edit can live in either a login file
/// (`.zprofile`, `.profile`) or an interactive one (`.zshrc`) — nvm's installer
/// writes to the latter — and a shell started with only one of the two flags
/// reads only half of what the user configured.
///
/// Bounded, and abandoned rather than waited on. A profile that opens an ssh
/// agent or prints a banner can take a while, and the editor must not stall on
/// it: after the deadline we take the static list and carry on. The worker
/// thread still reaps the child, so nothing is left behind.
fn login_shell_path() -> Option<OsString> {
    use std::sync::mpsc;
    use std::time::Duration;

    let shell = std::env::var_os("SHELL").unwrap_or_else(|| OsString::from("/bin/zsh"));
    // A shell we cannot run is not a shell. This also keeps the spawn off a
    // path the user could not have configured anything in.
    if !Path::new(&shell).is_file() {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let out = std::process::Command::new(&shell)
            .args(["-l", "-i", "-c", "printf %s \"$PATH\""])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output();
        let _ = tx.send(out.ok().filter(|o| o.status.success()).map(|o| o.stdout));
    });
    let bytes = rx.recv_timeout(Duration::from_millis(1500)).ok()??;
    let text = String::from_utf8(bytes).ok()?;
    let text = text.trim();
    // An interactive shell can print a banner before the value. Take the last
    // line, which is what `printf` wrote, and require it to look like a PATH.
    let line = text.lines().last()?.trim();
    (!line.is_empty() && line.contains('/')).then(|| OsString::from(line))
}

/// Every directory to look in, in order: the inherited `PATH` first (a user who
/// launched from a terminal, or set one deliberately, gets exactly what they
/// asked for), then what the login shell knows, then the standard locations
/// that Finder's environment omits.
fn search_dirs() -> &'static [PathBuf] {
    static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    DIRS.get_or_init(|| {
        let mut out: Vec<PathBuf> = Vec::new();
        if let Some(path) = std::env::var_os("PATH") {
            out.extend(std::env::split_paths(&path));
        }
        // Second, not first: a `PATH` deliberately set for this process — by a
        // terminal launch, or by a wrapper script — outranks whatever the
        // user's profile would have said.
        if let Some(shell_path) = login_shell_path() {
            for p in std::env::split_paths(&shell_path) {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        for dir in EXTRA_DIRS {
            if let Some(p) = expand(dir) {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        for p in node_version_dirs() {
            if !out.contains(&p) {
                out.push(p);
            }
        }
        // Last: a tool the user installed themselves outranks Apple's copy.
        for p in developer_dirs() {
            if !out.contains(&p) {
                out.push(p);
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
    if let Some(hit) = search_dirs()
        .iter()
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
    {
        return Some(hit);
    }
    // Only on a MISS, and only then: rescan the version-manager directories.
    //
    // `search_dirs` is a `OnceLock`, which is right for a list of fixed paths —
    // a directory created later is still searched, because the lookup stats the
    // candidate rather than the directory. The versioned roots are different:
    // they are DISCOVERED by reading the filesystem, so a Node installed after
    // launch would be invisible until a restart.
    //
    // That is exactly the flow the Components page invites — copy the command,
    // run it in a terminal, press Refresh — and "Not Installed" after doing
    // what the page told you to do is the worst answer the page can give.
    // A miss already means the whole list came up empty, so one extra
    // directory read costs nothing anybody waits on.
    node_version_dirs()
        .into_iter()
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

    /// Apple's toolchain is not on `PATH` — not even in a terminal — and
    /// `lldb-dap`, the debug adapter for Rust, C and C++, lives only there on a
    /// stock Mac. Measured: `which lldb-dap` finds nothing while the file
    /// exists under both Xcode and the Command Line Tools.
    #[test]
    fn the_developer_toolchain_is_searched() {
        let clt = Path::new("/Library/Developer/CommandLineTools/usr/bin");
        assert!(
            search_dirs().iter().any(|d| d == clt),
            "the Command Line Tools bin is searched: {:?}",
            search_dirs()
        );
    }

    /// And the toolchain `xcode-select` actually points at, which on a machine
    /// with both installed is not necessarily the one a hard-coded path would
    /// have guessed.
    #[test]
    fn the_selected_toolchain_is_searched_when_there_is_one() {
        let Ok(out) = Command::new("/usr/bin/xcode-select").arg("-p").output() else {
            return;
        };
        if !out.status.success() {
            return;
        }
        let selected = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if selected.is_empty() {
            return;
        }
        let bin = PathBuf::from(selected).join("usr/bin");
        assert!(
            search_dirs().contains(&bin),
            "the selected toolchain is searched: {bin:?}"
        );
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
