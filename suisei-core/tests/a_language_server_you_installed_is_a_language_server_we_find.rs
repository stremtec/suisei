//! Where a JavaScript toolchain actually lives, and what happens when it doesn't.
//!
//! `typescript-language-server` is installed with `npm i -g` — the line this
//! editor prints when it is missing — and on a developer's Mac, Node itself
//! usually comes from a version manager rather than from Homebrew. nvm, fnm,
//! Volta, pnpm and asdf all put their binaries somewhere the launcher's `PATH`
//! has never heard of, and a Finder-launched app inherits only
//! `/usr/bin:/bin:/usr/sbin:/sbin`.
//!
//! So Suisei concluded the server was not installed on machines that had it,
//! and hover and ⌘-click did nothing **and said nothing** — which is
//! indistinguishable from a broken feature. This is the exact bug `exec` was
//! written to end, recurring for a different toolchain.
//!
//! ```text
//! cargo test -p suisei-core --test a_language_server_you_installed_is_a_language_server_we_find
//! ```

use suisei_core::exec;

#[test]
fn the_search_covers_the_node_version_managers() {
    // A tripwire on the LIST, not on this machine: whether any of these exist
    // here says nothing, and a test that skipped when they were absent would
    // pass forever on a machine that never had them.
    let joined = exec::child_path().to_string_lossy().to_string();
    for needle in ["/.volta/bin", "/Library/pnpm", "/.npm-global/bin", "/.asdf/shims"] {
        assert!(
            joined.contains(needle),
            "{needle} is not searched — a tool installed there is invisible"
        );
    }
}

#[test]
fn the_search_still_covers_the_older_toolchains() {
    // The Node entries were appended; nothing that already worked may have
    // been displaced by them.
    let joined = exec::child_path().to_string_lossy().to_string();
    for needle in ["/opt/homebrew/bin", "/usr/local/bin", "/.cargo/bin", "/.local/bin"] {
        assert!(joined.contains(needle), "{needle} stopped being searched");
    }
}

#[test]
fn a_child_process_is_handed_the_same_list_we_searched() {
    // `typescript-language-server` is a Node script: it finds `node` on its own
    // `PATH`. Locating the server through a directory we never hand down would
    // start a process that immediately fails to find its own interpreter.
    let joined = exec::child_path().to_string_lossy().to_string();
    assert!(!joined.is_empty());
    for dir in joined.split(':') {
        assert!(!dir.is_empty(), "an empty entry means a `PATH` join went wrong");
    }
}

#[test]
fn a_name_with_a_separator_is_a_path_and_not_a_search() {
    // Settings lets a user point at a server by absolute path. That must not be
    // looked up in the directories above and silently resolved to a different
    // binary with the same name.
    assert_eq!(exec::find("/bin/sh"), Some(std::path::PathBuf::from("/bin/sh")));
    assert_eq!(exec::find("/definitely/not/here/tsserver"), None);
}

#[test]
fn something_that_is_not_installed_is_still_reported_as_not_installed() {
    // The fix widens the search; it must not make the search always succeed.
    assert!(exec::find("suisei-nonexistent-language-server").is_none());
}
