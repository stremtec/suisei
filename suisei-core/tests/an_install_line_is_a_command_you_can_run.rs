//! Everything the Components page prints must be a command, or a link.
//!
//! The page grew an Install button that RUNS the string, and the table it runs
//! was written as prose in places: `zls` said "see https://github.com/…" and
//! `dart` said "install the Dart SDK". Both were handed to a shell. Two more
//! named packages that do not exist on macOS at all — `brew install lemminx`
//! and `npm i -g js-debug-adapter`, which the npm registry answers 404 to — so
//! the command ran, failed, and the user was told to install something that
//! cannot be installed that way.
//!
//! This is the shape of that class of bug, caught in the table rather than in
//! the report. It cannot check that a package EXISTS — that needs a network
//! and a registry, and was done by hand with `brew info` / `npm view` when
//! these were written — but it can insist that every line is a command, that
//! every row with no command has somewhere to send the user instead, and that
//! nothing is prose pretending to be either.

use suisei_core::config::lsp_lang_catalog;
use suisei_core::lsp::{install_command, install_docs};

/// The programs an install line may start with. A closed set on purpose: a new
/// one is a decision — "Suisei now tells people to run this" — and it should be
/// made here rather than by a string appearing in the table.
const INSTALLERS: &[&str] = &[
    "brew", "npm", "go", "cargo", "rustup", "pip3", "python3", "gem", "dotnet", "nimble", "pipx",
    "ghcup", "xcode-select", "R", "cs",
];

fn first_word(s: &str) -> &str {
    s.split_whitespace().next().unwrap_or("")
}

#[test]
fn every_language_server_offers_a_command_or_a_link() {
    for (key, label, command) in lsp_lang_catalog() {
        let bin = command.split_whitespace().next().unwrap_or(command);
        let install = install_command(bin);

        if install.is_empty() {
            let docs = install_docs(bin);
            assert!(
                docs.starts_with("https://"),
                "{label} ({key}) has no install command and no link — the page \
                 would show a missing server and no way to get it"
            );
            continue;
        }

        assert!(
            INSTALLERS.contains(&first_word(install)),
            "{label} ({key}) installs with `{install}`, which does not start \
             with a known installer. A settings page runs this line."
        );
        // The two shapes the prose took. Both were live.
        assert!(
            !install.contains("see http") && !install.contains("://"),
            "{label} ({key}) has a URL in its install command: `{install}`. A \
             link belongs in `install_docs`, where the page draws it as one."
        );
        assert!(
            !install.contains("install the "),
            "{label} ({key}) has a sentence, not a command: `{install}`"
        );
    }
}

/// A link is for the tools nothing packages. Anything a package manager carries
/// must not have one, or the row offers two answers to one question.
#[test]
fn a_link_and_a_command_are_never_both_offered() {
    for (_key, label, command) in lsp_lang_catalog() {
        let bin = command.split_whitespace().next().unwrap_or(command);
        if install_command(bin).is_empty() {
            continue;
        }
        assert!(
            install_docs(bin).is_empty(),
            "{label} has both an install command and a link"
        );
    }
}

/// The two that are genuinely unpackaged on macOS, named. If one of them ever
/// ships a formula or a package, this is where the news arrives.
#[test]
fn the_unpackaged_ones_are_the_ones_we_know_about() {
    assert!(install_command("lemminx").is_empty());
    assert!(install_docs("lemminx").starts_with("https://"));
    assert!(install_command("js-debug-adapter").is_empty());
    assert!(install_docs("js-debug-adapter").starts_with("https://"));

    // …and everything the user reported as broken alongside them now has one.
    for bin in [
        "zls",
        "dart",
        "haskell-language-server-wrapper",
        "nimlsp",
        "cmake-language-server",
    ] {
        let cmd = install_command(bin);
        assert!(
            INSTALLERS.contains(&first_word(cmd)),
            "{bin} still has no runnable install line (`{cmd}`)"
        );
    }
}
