//! What Suisei is made of, and what it needs from the machine.
//!
//! Xcode's Components page, answered honestly. The measurement in
//! `docs/SUISEI-COMPONENTS-PLAN.md` moved this a long way from where the
//! question started:
//!
//!   · **The debugger is 0 MB.** Rust, C, C++ and Objective-C debug through
//!     `lldb-dap`, which ships inside Xcode and inside the Command Line Tools.
//!     Python, Go and Node debug through `debugpy`, `dlv` and
//!     `js-debug-adapter`, which come from pip, go and npm — where their users
//!     already get everything else, and where the security updates come from.
//!     Hosting our own copies would mean shipping a second debugger beside the
//!     one Apple already updates, and taking on its CVEs. The same argument
//!     covers language servers.
//!
//!   · So the component for debugging is **not a binary. It is finding what is
//!     there, and helping install what is not.** That is this file.
//!
//! And it is the half of the feature that WORKS today. Downloading a grammar
//! into the process needs a signed, notarized release first — the hardened
//! runtime's library validation refuses a dylib signed by anyone else — and
//! that pipeline does not exist yet. Detection needs none of it.
//!
//! **`crate::exec`, never `PATH`.** An app launched from Finder inherits
//! `/usr/bin:/bin:/usr/sbin:/sbin`, so Homebrew, cargo, and Apple's own
//! toolchain are all invisible to a naive lookup. That is not hypothetical:
//! Suisei told users to install `lldb-dap` on Macs that had it twice over, and
//! `exec` exists because of it. A page whose whole job is to report what is
//! installed must not reintroduce the bug it is reporting on.

use std::path::PathBuf;

/// Whether a component is here, and how it got here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// Compiled into the app. Nothing to install, nothing to download.
    Bundled,
    /// Found on this machine, at this path.
    ///
    /// The path is carried because WHICH copy answered is the useful fact. A
    /// developer Mac routinely has three `clangd`s and a stale one first.
    Present(PathBuf),
    /// Not here. `install` is the line that gets it.
    Missing,
}

impl Availability {
    pub fn is_present(&self) -> bool {
        !matches!(self, Availability::Missing)
    }
}

/// One row of the Components page.
#[derive(Debug, Clone)]
pub struct Component {
    pub id: String,
    pub title: String,
    /// Which section it belongs to; see [`GROUPS`] for the drawn order.
    pub group: &'static str,
    /// What it does, or which languages it covers.
    pub detail: String,
    /// The shell line that installs it. Empty when there is nothing to run.
    pub install: String,
    /// Where the builds are published, for the components no package manager
    /// carries. Empty whenever `install` is a command — one answer per row.
    pub docs: String,
    pub state: Availability,
}

/// Section order, top to bottom. Actionable first: a row with a command the
/// user can run is worth more of the page than a row that only says "included".
pub const GROUPS: &[&str] = &["Debugging", "Language Servers", "Included"];

/// Why nothing here can be downloaded yet.
///
/// One owner for the fact, so the page and any future install gate cannot
/// disagree about it. Returns empty once signed releases exist.
pub fn download_blocked_reason() -> &'static str {
    "Downloadable components need a signed, notarized release, and that is not \
     published yet. Everything Suisei ships is included in the app below."
}

/// How a component's presence is decided.
enum Probe {
    /// Any one of these binaries, in order. The first found wins, which is why
    /// the preferred adapter is listed first.
    Binary(&'static [&'static str]),
    /// A Python module, asked of whichever `python3` is on the machine.
    ///
    /// `debugpy` installs no console script, so there is no binary to look for
    /// — the only honest question is whether the interpreter can import it.
    /// Costs one interpreter start (~100 ms) per refresh, which is why the page
    /// pulls this off the main thread and only when it is opened.
    PythonModule(&'static str),
    /// Shipped inside the app.
    Builtin,
}

struct Row {
    id: &'static str,
    title: &'static str,
    group: &'static str,
    detail: &'static str,
    install: &'static str,
    /// See [`Component::docs`]. Empty unless `install` is.
    docs: &'static str,
    probe: Probe,
}

const ROWS: &[Row] = &[
    Row {
        id: "dap.lldb",
        title: "LLDB",
        group: "Debugging",
        detail: "Rust, C, C++, Objective-C and Swift. Ships inside Xcode and the Command Line Tools.",
        install: "xcode-select --install",
        docs: "",
        probe: Probe::Binary(&["lldb-dap", "codelldb", "lldb-vscode"]),
    },
    Row {
        id: "dap.debugpy",
        title: "debugpy",
        group: "Debugging",
        detail: "Python.",
        // Filled in from the interpreter we actually probe — see
        // `python_install`. A fixed `pip3 install debugpy` is wrong twice over.
        install: "",
        docs: "",
        probe: Probe::PythonModule("debugpy"),
    },
    Row {
        id: "dap.delve",
        title: "Delve",
        group: "Debugging",
        detail: "Go.",
        install: "go install github.com/go-delve/delve/cmd/dlv@latest",
        docs: "",
        probe: Probe::Binary(&["dlv"]),
    },
    Row {
        id: "dap.js",
        title: "js-debug",
        group: "Debugging",
        detail: "Node.js and JavaScript. No packaged build for macOS — unpack a \
release and point Settings at its `dapDebugServer.js`.",
        // `npm i -g js-debug-adapter` was printed here and **the package does
        // not exist**: the registry answers 404. Microsoft publishes the DAP
        // as a release tarball and nothing else, on npm or in Homebrew, so
        // there is no line to run and the row says so with a link.
        install: "",
        docs: "https://github.com/microsoft/vscode-js-debug/releases",
        probe: Probe::Binary(&["js-debug-adapter"]),
    },
    Row {
        id: "inc.syntax",
        title: "Syntax Highlighting",
        group: "Included",
        // Filled in from `Lang::ALL` — the list has to come from the grammars
        // actually compiled in, or the page is describing a different build.
        detail: "",
        install: "",
        docs: "",
        probe: Probe::Builtin,
    },
    Row {
        id: "inc.models",
        title: "3D Model Viewer",
        group: "Included",
        detail: "glTF, GLB and FBX open in a viewer rather than as binary text.",
        install: "",
        docs: "",
        probe: Probe::Builtin,
    },
    Row {
        id: "inc.terminal",
        title: "Terminal",
        group: "Included",
        detail: "A real PTY, in a tab or beside the editor.",
        install: "",
        docs: "",
        probe: Probe::Builtin,
    },
];

/// Every row, with this machine's answer for each.
///
/// Built fresh on each call rather than cached: a user who opens this page has
/// usually just installed something, and a cache would show them the answer
/// from before they did.
pub fn catalog() -> Vec<Component> {
    let mut out: Vec<Component> = Vec::with_capacity(ROWS.len() + 16);

    for row in ROWS {
        let state = resolve(&row.probe);
        let detail = if row.id == "inc.syntax" {
            languages_line()
        } else {
            row.detail.to_string()
        };
        let install = if row.id == "dap.debugpy" {
            python_install("debugpy")
        } else {
            row.install.to_string()
        };
        out.push(Component {
            id: row.id.to_string(),
            title: row.title.to_string(),
            group: row.group,
            detail,
            install,
            docs: row.docs.to_string(),
            state,
        });
    }

    // Language servers come from the SAME table Settings configures them from
    // (`config::lsp_lang_catalog`), not a second list beside it. A page that
    // reported on servers the editor does not actually start would be a page
    // that can be wrong without anything failing.
    for (key, label, command) in crate::config::lsp_lang_catalog() {
        let bin = command.split_whitespace().next().unwrap_or(command);
        out.push(Component {
            id: format!("lsp.{key}"),
            title: (*label).to_string(),
            group: "Language Servers",
            detail: (*command).to_string(),
            install: crate::lsp::install_command(bin).to_string(),
            docs: crate::lsp::install_docs(bin).to_string(),
            state: match crate::exec::find(bin) {
                Some(p) => Availability::Present(p),
                None => Availability::Missing,
            },
        });
    }

    out
}

/// The line that installs a Python package **into the interpreter we probe**.
///
/// Two things were wrong with the fixed `pip3 install debugpy`:
///
///   · **`pip3` need not belong to the `python3` we ask.** The probe imports
///     the module with whichever `python3` `exec` finds. A machine with more
///     than one Python installs into one and is then asked about another, and
///     the page still says Not Installed after the user did exactly what it
///     said. `python3 -m pip` cannot drift: it is the same interpreter.
///
///   · **PEP 668.** A Homebrew or distro Python ships an `EXTERNALLY-MANAGED`
///     marker and refuses `pip install` outright — `--user` too. Measured on
///     this machine: `error: externally-managed-environment`. So the page was
///     printing a command that CANNOT WORK, and the user is left thinking the
///     detection is broken when it is the instruction that is.
///
/// The marker is read from the interpreter's own `stdlib` path rather than
/// guessed from the binary's location: a venv, pyenv build and Homebrew build
/// all sit somewhere different and only the interpreter knows which it is.
fn python_install(package: &str) -> String {
    let Some(py) = crate::exec::find("python3").or_else(|| crate::exec::find("python")) else {
        // Nothing to install into. Name the tool anyway — a user with no
        // python3 needs to hear that first.
        return format!("python3 -m pip install {package}");
    };
    let managed = std::process::Command::new(&py)
        .arg("-c")
        .arg("import os,sysconfig;print(os.path.exists(os.path.join(sysconfig.get_path('stdlib'),'EXTERNALLY-MANAGED')))")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "True")
        .unwrap_or(false);
    if managed {
        // What actually works on such an install. Homebrew's Python is not
        // macOS's, so this risks Homebrew's own packages and not the OS — the
        // alternative honest answer is a virtualenv, and a debug adapter has to
        // be importable by the interpreter that runs the code, which for now is
        // this one.
        format!("python3 -m pip install --break-system-packages {package}")
    } else {
        format!("python3 -m pip install {package}")
    }
}

/// The grammars this build actually has, named.
fn languages_line() -> String {
    let names: Vec<&str> = crate::lang::Lang::ALL.iter().map(|l| l.name()).collect();
    format!("{} languages: {}.", names.len(), names.join(", "))
}

fn resolve(probe: &Probe) -> Availability {
    match probe {
        Probe::Builtin => Availability::Bundled,
        Probe::Binary(names) => names
            .iter()
            .find_map(|n| crate::exec::find(n))
            .map(Availability::Present)
            .unwrap_or(Availability::Missing),
        Probe::PythonModule(module) => python_module(module),
    }
}

/// Whether some `python3` on this machine can import `module`.
///
/// The interpreter is found through `exec` for the same reason everything else
/// here is: a Finder-launched app cannot see Homebrew's or pyenv's python, and
/// reporting "Python debugging unavailable" to someone who has had debugpy for
/// years is the exact failure this module exists to stop repeating.
fn python_module(module: &str) -> Availability {
    let Some(py) = crate::exec::find("python3").or_else(|| crate::exec::find("python")) else {
        return Availability::Missing;
    };
    let ok = std::process::Command::new(&py)
        .arg("-c")
        .arg(format!("import {module}"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        // The INTERPRETER's path, because that is the copy that will run it —
        // a module has no path of its own that means anything to a user.
        Availability::Present(py)
    } else {
        Availability::Missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_well_formed() {
        for c in catalog() {
            assert!(!c.id.is_empty(), "a row with no id");
            assert!(!c.title.is_empty(), "{} has no title", c.id);
            assert!(
                GROUPS.contains(&c.group),
                "{} is in group {:?}, which the page does not draw",
                c.id,
                c.group
            );
        }
    }

    #[test]
    fn ids_are_unique() {
        let all = catalog();
        let mut ids: Vec<&str> = all.iter().map(|c| c.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "two rows share an id");
    }

    #[test]
    fn a_missing_component_always_says_how_to_get_it() {
        // The rule the page exists for. A row that reports "not installed" and
        // stops is worth less than no row at all: it names a problem and hands
        // back nothing.
        //
        // A command OR a link, since two of these have no macOS package and
        // used to satisfy this test with a line that could not work — `brew
        // install lemminx`, `npm i -g js-debug-adapter`. "Says how to get it"
        // was always the rule; only `install` being the sole way to say it was
        // ever the assumption.
        for c in catalog() {
            if c.state == Availability::Missing {
                assert!(
                    !c.install.is_empty() || !c.docs.is_empty(),
                    "{} is missing and offers neither a command nor a link",
                    c.id
                );
            }
        }
    }

    #[test]
    fn a_bundled_component_has_nothing_to_install() {
        for c in catalog() {
            if c.state == Availability::Bundled {
                assert!(c.install.is_empty(), "{} ships with the app", c.id);
            }
        }
    }

    #[test]
    fn the_language_server_rows_come_from_the_configured_table() {
        let all = catalog();
        for (key, label, _) in crate::config::lsp_lang_catalog() {
            let id = format!("lsp.{key}");
            let row = all.iter().find(|c| c.id == id);
            assert!(row.is_some(), "{label} is configurable but not listed");
        }
    }

    #[test]
    fn the_included_row_names_the_grammars_this_build_has() {
        let line = languages_line();
        // Not a hardcoded count: the assertion is that the row and the build
        // agree, which is the only property worth holding.
        assert!(line.starts_with(&format!("{} languages:", crate::lang::Lang::ALL.len())));
        assert!(line.contains("Rust"), "{line}");
    }

    #[test]
    fn nothing_is_downloadable_yet_and_the_page_says_why() {
        assert!(!download_blocked_reason().is_empty());
    }

    #[test]
    fn presence_is_resolved_through_exec_not_path() {
        // `sh` is on every machine and in `/bin`, which a Finder-launched app
        // DOES inherit — so this asserts the lookup works at all, not that it
        // beats PATH. The real guard is that `resolve` has no other route to
        // an answer; there is no `std::env::var("PATH")` in this file.
        let found = resolve(&Probe::Binary(&["sh"]));
        assert!(found.is_present(), "sh was not found: {found:?}");
        // The needle is BUILT rather than written, so this test's own source
        // does not contain the thing it is looking for — the first version of
        // this line spelled the pattern out in a comment and failed on itself.
        // It catches env::var, env::var_os and split_paths alike, because all
        // three name the variable as a quoted literal.
        let path_literal = format!("{}PATH{}", '"', '"');
        // Comment lines are dropped first. Without that this watches PROSE:
        // a comment explaining the rule trips the check that enforces it, and
        // the way to keep the suite green becomes deleting the explanation.
        let code: String = std::include_str!("components.rs")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !code.contains(&path_literal),
            "this module reads PATH directly — that is the bug `exec` exists to end"
        );
    }
}
