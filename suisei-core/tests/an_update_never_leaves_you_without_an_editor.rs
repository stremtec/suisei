//! The failure model of the source updater.
//!
//! One rule: **there is a working Suisei on disk at every instant.** Not
//! "almost always", and not "unless the power goes out during the swap". Every
//! test here is a way that could stop being true.
//!
//! The dangerous step is the swap. `rm -rf old && mv new old` has a window
//! between the two commands where the user owns no editor, and a crash there is
//! unrecoverable without a repair path. `renamex_np(RENAME_SWAP)` exchanges two
//! directory entries atomically: before the call the app path holds the old
//! build, after it the new one, and never nothing. It also leaves the old
//! bundle where the new one was, which turns rolling back into a rename.
//!
//! ```text
//! cargo test -p suisei-core --test an_update_never_leaves_you_without_an_editor
//! ```

use std::path::PathBuf;
use suisei_core::update_build::{self as ub, Pending};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("suisei_update_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// A bundle just real enough for the checks that matter.
fn make_app(at: &PathBuf, marker: &str) {
    let macos = at.join("Contents/MacOS");
    std::fs::create_dir_all(&macos).expect("bundle");
    std::fs::write(macos.join("Suisei"), marker).expect("executable");
}

// ── The swap ────────────────────────────────────────────────────────────────

#[test]
fn the_swap_exchanges_both_ways_at_once() {
    let dir = scratch("swap");
    let (new, app) = (dir.join("staged.app"), dir.join("Suisei.app"));
    make_app(&new, "NEW");
    make_app(&app, "OLD");

    ub::swap(&new, &app).expect("swap");

    assert_eq!(read(&app), "NEW", "the app path holds the new build");
    assert_eq!(read(&new), "OLD", "and the old one is still here");
}

#[test]
fn the_old_build_is_the_rollback_not_a_leftover() {
    // Putting it back is another swap. That is the whole recovery story, and
    // it is why the old bundle is kept rather than deleted.
    let dir = scratch("rollback");
    let (new, app) = (dir.join("staged.app"), dir.join("Suisei.app"));
    make_app(&new, "NEW");
    make_app(&app, "OLD");

    ub::swap(&new, &app).expect("forward");
    ub::swap(&new, &app).expect("back");
    assert_eq!(read(&app), "OLD", "rolled back");
}

#[test]
fn a_swap_that_cannot_happen_changes_nothing() {
    // The failure mode that matters: if the exchange cannot be made, the
    // installed app must be untouched rather than half-replaced.
    let dir = scratch("swapfail");
    let app = dir.join("Suisei.app");
    make_app(&app, "OLD");
    let missing = dir.join("not-here.app");

    assert!(ub::swap(&missing, &app).is_err());
    assert_eq!(read(&app), "OLD", "the installed app survived");
}

fn read(app: &PathBuf) -> String {
    std::fs::read_to_string(app.join("Contents/MacOS/Suisei")).unwrap_or_default()
}

// ── The marker that decides whether to swap at all ──────────────────────────

#[test]
fn the_marker_survives_a_round_trip() {
    let p = Pending {
        version: "0.2.0".into(),
        sha: "abc123".into(),
        app: PathBuf::from("/tmp/staged-0.2.0/Suisei.app"),
    };
    assert_eq!(Pending::parse(&p.serialise()), Some(p));
}

#[test]
fn anything_unexpected_in_the_marker_means_do_nothing() {
    // This file decides whether the editor replaces itself at startup. A
    // truncated or hand-edited one must read as "no update", never as "do
    // something with the fields I could make out".
    for bad in [
        "",
        "version=0.2.0\n",
        "version=0.2.0\nsha=abc\n",
        "version=\nsha=abc\napp=/x\n",
        "version=0.2.0\nsha=abc\napp=\n",
        "garbage",
        "version=0.2.0\nsha=abc\napp=/x\nextra=1\n",
    ] {
        assert_eq!(Pending::parse(bad), None, "accepted {bad:?}");
    }
}

// ── What the next launch decides ───────────────────────────────────────────

#[test]
fn a_staged_app_that_vanished_is_not_applied() {
    // Everything the updater writes lives under Caches, which the system and
    // the user are both entitled to sweep. The marker outliving the bundle is
    // an ordinary Tuesday, not a corruption.
    let dir = scratch("vanished");
    let marker = dir.join("pending");
    let p = Pending {
        version: "9.9.9".into(),
        sha: "abc".into(),
        app: dir.join("gone/Suisei.app"),
    };
    std::fs::write(&marker, p.serialise()).expect("write marker");

    assert!(
        ub::pending_at(&marker, "0.1.0").is_none(),
        "applied a missing bundle"
    );
}

#[test]
fn a_marker_for_the_version_already_running_is_not_applied() {
    // The marker outlived its own update — it was applied and the file was not
    // cleaned up, or the same version was staged twice. Applying it would swap
    // the app for an identical one on every launch, forever.
    let dir = scratch("already");
    let app = dir.join("Suisei.app");
    make_app(&app, "NEW");
    let marker = dir.join("pending");
    std::fs::write(
        &marker,
        Pending {
            version: "0.2.0".into(),
            sha: "abc".into(),
            app,
        }
        .serialise(),
    )
    .expect("write marker");

    assert!(
        ub::pending_at(&marker, "0.2.0").is_none(),
        "swapped onto itself"
    );
}

#[test]
fn no_marker_at_all_is_the_ordinary_case() {
    let dir = scratch("nomarker");
    assert!(ub::pending_at(&dir.join("pending"), "0.1.0").is_none());
}

// ── Refusing before spending anything ──────────────────────────────────────

#[test]
fn a_build_is_refused_when_the_release_has_no_build_script() {
    // Not a hypothetical: an older tag predates `scripts/release.sh`. Guessing
    // how to build it would run something unknown; saying so costs a clone and
    // stops there.
    let dir = scratch("noscript");
    let out = ub::run("0.0.1", "", &format!("file://{}", dir.display()), &|_| {});
    assert!(out.is_err(), "built something it should not have");
}
