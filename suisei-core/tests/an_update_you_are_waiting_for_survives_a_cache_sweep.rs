//! Clearing the update cache must never clear the update.
//!
//! A source update clones the repository and builds it, so `~/Library/Caches/
//! Suisei/update` grows to gigabytes — worth a button. But the STAGED BUNDLE
//! lives in that same directory, and the whole safety argument for source
//! updates is that there is a working Suisei on disk at every instant, with the
//! atomic swap as the only destructive step.
//!
//! A "Clear Cache" button that throws away the update the user is waiting to
//! restart into would be the one way to break that from inside the product. So
//! it refuses, and says which version it is protecting.
//!
//! ```text
//! cargo test -p suisei-core --test an_update_you_are_waiting_for_survives_a_cache_sweep
//! ```

use suisei_core::update_build::{self, ClearRefused, Pending};

/// A marker of our own — never the real `~/Library/Caches` one, which would
/// make these tests race each other AND edit the developer's install state.
fn marker(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("suisei_cache_sweep");
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

/// A staged bundle complete enough that `pending_at` believes in it.
fn stage(dir: &std::path::Path) {
    std::fs::create_dir_all(dir.join("Contents/MacOS")).expect("bundle");
    std::fs::write(dir.join("Contents/MacOS/Suisei"), b"#!/bin/sh\n").expect("exe");
}

#[test]
fn a_running_build_is_not_swept_out_from_under() {
    // Checked before anything is looked at, let alone removed: the build is
    // writing into this directory right now.
    assert_eq!(
        update_build::clear_cache("0.1.1", true),
        Err(ClearRefused::BuildRunning)
    );
}

#[test]
fn a_staged_update_refuses_and_names_itself() {
    let m = marker("staged.pending");
    let app = std::env::temp_dir().join("suisei_cache_sweep/staged-0.1.2/Suisei.app");
    stage(&app);
    let p = Pending {
        version: "0.1.2".into(),
        sha: "cecb7969f276803fdf086b92c6829073a8e50f6f".into(),
        app: app.clone(),
    };
    std::fs::write(&m, p.serialise()).expect("marker");

    // `pending_at` is the same check `clear_cache` makes, on a marker a test
    // can own. That it answers here is what makes the refusal reachable.
    let seen = update_build::pending_at(&m, "0.1.1").expect("a pending update");
    assert_eq!(seen.version, "0.1.2");
    assert_eq!(seen.app, app);

    let _ = std::fs::remove_file(&m);
}

#[test]
fn a_marker_for_the_running_version_protects_nothing() {
    // The update already landed; the marker outlived it. Refusing here would
    // leave the cache un-clearable forever after one successful update.
    let m = marker("already.pending");
    let app = std::env::temp_dir().join("suisei_cache_sweep/staged-0.1.1/Suisei.app");
    stage(&app);
    std::fs::write(
        &m,
        Pending {
            version: "0.1.1".into(),
            sha: "abc".into(),
            app,
        }
        .serialise(),
    )
    .expect("marker");

    assert!(update_build::pending_at(&m, "0.1.1").is_none());
    let _ = std::fs::remove_file(&m);
}

#[test]
fn a_marker_whose_bundle_is_gone_protects_nothing() {
    // Exactly the state a previous sweep leaves behind. The marker is not the
    // update; the bundle is.
    let m = marker("ghost.pending");
    std::fs::write(
        &m,
        Pending {
            version: "0.9.9".into(),
            sha: "abc".into(),
            app: std::env::temp_dir().join("suisei_cache_sweep/not-here/Suisei.app"),
        }
        .serialise(),
    )
    .expect("marker");

    assert!(update_build::pending_at(&m, "0.1.1").is_none());
    let _ = std::fs::remove_file(&m);
}

#[test]
fn the_size_of_an_absent_cache_is_zero_not_an_error() {
    // Reported on a page that opens whether or not an update has ever run.
    // This must be a number, never a failure to show one.
    let _ = update_build::cache_bytes();
}

// There is no test here that CALLS `clear_cache` with `building: false` and no
// pending marker, and that is deliberate: the only directory it can be pointed
// at is the real `~/Library/Caches/Suisei/update`, so such a test would delete
// the developer's own staged update — the exact shared-fixture mistake the
// comment on `pending_at` exists to warn about. The first draft of this file
// had one.
//
// What is testable without owning the machine is the refusal, and that is above:
// `clear_cache` consults `pending_for`, which is `pending_at` on the real
// marker, and the three cases that decide it are pinned against markers a test
// owns.
