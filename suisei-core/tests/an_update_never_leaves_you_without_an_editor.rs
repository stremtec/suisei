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

// ── The percentage, and whether it is telling the truth ────────────────────

use suisei_core::update_build::ProgressModel;

/// The step lines a real cold build prints, in order. Taken from the release
/// script and packager's own output, not invented.
fn feed(model: &mut ProgressModel, lines: &[&str]) {
    for l in lines {
        model.observe(l);
    }
}

#[test]
fn the_bar_only_goes_forwards() {
    // The one property a progress bar cannot violate and still be believed.
    let mut m = ProgressModel::new(81);
    let script = [
        "▸ Welcome art",
        "  19M masters → 6.2M shipped",
        "→ SwiftTerm (vendored, pinned)",
        "Compiling swift-term",
        "→ cargo build -p suisei-engine --release",
        "Compiling libc v0.2",
        "Compiling serde v1.0",
        "Compiling suisei-core v0.1.0",
        "→ swiftc → Contents/MacOS/Suisei",
        "→ embed GLTFKit2.framework",
        "→ build + bundle daemon",
        "→ packaged /x/Suisei.app",
    ];
    let mut last = 0.0f32;
    for line in script {
        m.observe(line);
        let f = m.progress(60).fraction;
        assert!(f >= last, "went backwards at {line:?}: {last} → {f}");
        last = f;
    }
    assert!(last > 0.9, "ended at {last}, should be near done");
}

#[test]
fn the_engine_step_is_counted_against_the_lock_file() {
    // The part that is real rather than estimated: one `Compiling` line per
    // package, and `Cargo.lock` says how many packages there are.
    let mut a = ProgressModel::new(100);
    feed(&mut a, &["→ cargo build -p suisei-engine --release"]);
    let start = a.progress(60).fraction;

    for i in 0..50 {
        a.observe(&format!("Compiling crate{i} v1.0"));
    }
    let half = a.progress(60).fraction;

    for i in 50..100 {
        a.observe(&format!("Compiling crate{i} v1.0"));
    }
    let full = a.progress(60).fraction;

    assert!(half > start && full > half, "{start} {half} {full}");
    // Half the crates should be about half of the engine step's share.
    let step = full - start;
    let got = half - start;
    assert!(
        (got - step / 2.0).abs() < 0.02,
        "half the crates moved {got}, step is {step}"
    );
}

#[test]
fn the_terminal_s_compiling_lines_do_not_count_against_the_engine() {
    // SwiftPM prints the same word. Counting those against the engine's
    // denominator would run the bar past its own step before the engine
    // started, and then it would have to stall or go backwards.
    let mut m = ProgressModel::new(10);
    feed(&mut m, &["→ SwiftTerm (vendored, pinned)"]);
    let before = m.progress(60).fraction;
    for i in 0..10 {
        m.observe(&format!("Compiling SwiftTermPart{i}"));
    }
    assert_eq!(m.progress(60).fraction, before, "counted the wrong step");
}

#[test]
fn it_never_claims_to_be_finished() {
    // The bar reaching 100% while the build is still running is the specific
    // lie that makes people force-quit.
    let mut m = ProgressModel::new(4);
    feed(&mut m, &["→ packaged /x/Suisei.app"]);
    for i in 0..99 {
        m.observe(&format!("Compiling x{i}"));
    }
    assert!(m.progress(9_999).fraction < 1.0);
}

#[test]
fn there_is_no_estimate_until_there_is_something_to_estimate_from() {
    // `elapsed / fraction` with either number tiny reports an hour on a build
    // with thirty seconds left. Saying nothing is the honest answer then.
    let mut m = ProgressModel::new(81);
    assert_eq!(m.progress(1).eta_secs, None, "estimated from one second");
    feed(&mut m, &["→ cargo build -p suisei-engine --release"]);
    m.observe("Compiling a v1.0");
    assert_eq!(m.progress(5).eta_secs, None, "estimated too early");

    // Far enough in, it answers.
    feed(&mut m, &["→ swiftc → Contents/MacOS/Suisei"]);
    assert!(m.progress(600).eta_secs.is_some());
}

#[test]
fn the_estimate_shrinks_as_the_build_goes_on() {
    let mut m = ProgressModel::new(81);
    feed(&mut m, &["→ cargo build -p suisei-engine --release"]);
    for i in 0..40 {
        m.observe(&format!("Compiling c{i} v1.0"));
    }
    let early = m.progress(300).eta_secs.expect("an estimate");
    feed(&mut m, &["→ swiftc → x", "→ build + bundle daemon"]);
    let late = m.progress(600).eta_secs.expect("an estimate");
    assert!(late < early, "estimate grew: {early} → {late}");
}

#[test]
fn an_unreadable_lock_file_degrades_to_no_granularity() {
    // The denominator is a divisor. A clone we could not read the lock file
    // from must lose the fine-grained part, not crash or divide by zero.
    let mut m = ProgressModel::new(0);
    feed(&mut m, &["→ cargo build -p suisei-engine --release"]);
    m.observe("Compiling a v1.0");
    let f = m.progress(60).fraction;
    assert!(f.is_finite() && (0.0..1.0).contains(&f), "got {f}");
}

#[test]
fn the_headline_says_what_it_is_doing_in_the_users_words() {
    let mut m = ProgressModel::new(81);
    feed(&mut m, &["→ cargo build -p suisei-engine --release"]);
    let h = m.progress(60).headline;
    assert!(h.contains("engine"), "{h}");
    assert!(!h.contains("cargo"), "leaked the build system's words: {h}");
}

// ── Calibrating from the last build ────────────────────────────────────────

use suisei_core::update_build::Calibration;

#[test]
fn timings_survive_a_round_trip() {
    let c = Calibration {
        secs: vec![(0, 3), (3, 400), (4, 600)],
    };
    assert_eq!(Calibration::parse(&c.serialise()), Some(c));
}

#[test]
fn half_a_set_of_timings_is_no_timings() {
    // Worse than the defaults, because it is wrong in a way that looks
    // measured — the bar would weight two steps from real numbers and the rest
    // from guesses, and land somewhere neither.
    for bad in ["", "0", "0=", "=5", "x=5", "0=abc", "0=3\nbroken"] {
        assert_eq!(Calibration::parse(bad), None, "accepted {bad:?}");
    }
}

#[test]
fn the_swift_step_moves_once_it_has_been_measured_before() {
    // The point of the whole mechanism. `swiftc -whole-module-optimization`
    // emits ONE compile job — verified with `-parseable-output`, which reports
    // exactly two events for the module — so there is nothing to count inside
    // it. With a previous build's duration, it can be timed instead.
    let cal = Calibration {
        secs: vec![(0, 1), (1, 1), (2, 60), (3, 300), (4, 600), (5, 30), (6, 5)],
    };
    let mut m = ProgressModel::with_calibration(81, cal);
    m.tick(400);
    m.observe("→ swiftc → Contents/MacOS/Suisei");

    m.tick(400);
    let at_start = m.progress(400).fraction;
    m.tick(700);
    let halfway = m.progress(700).fraction;

    assert!(halfway > at_start, "the bar stalled: {at_start} → {halfway}");
}

#[test]
fn a_timed_step_never_reaches_its_own_end() {
    // A clock can run out before the work does. A bar parked at "this step is
    // finished" while the step is still running is the same lie as one that
    // finishes early — and the Swift step is exactly where an estimate is
    // most likely to be short.
    let cal = Calibration {
        secs: vec![(0, 1), (4, 100)],
    };
    let mut m = ProgressModel::with_calibration(81, cal);
    m.tick(0);
    m.observe("→ swiftc → x");
    let entered = m.progress(0).fraction;
    m.tick(100_000);
    let overrun = m.progress(100_000).fraction;

    // It advanced through the step, and stopped short of consuming all of it —
    // the steps after Swift still have room, which is what says "not finished".
    assert!(overrun > entered, "the bar stalled");
    assert!(
        overrun < 0.99,
        "a clock that ran out claimed the build was done: {overrun}"
    );
}

#[test]
fn with_nothing_measured_the_bar_holds_still_rather_than_guessing() {
    // First update on a machine. Better a bar that pauses honestly than one
    // that creeps on a guess and arrives before the work does.
    let mut m = ProgressModel::new(81);
    m.tick(0);
    m.observe("→ swiftc → x");
    let a = m.progress(10).fraction;
    m.tick(9_000);
    let b = m.progress(9_000).fraction;
    assert_eq!(a, b, "invented movement with nothing to go on");
}

#[test]
fn a_build_reports_what_each_step_cost() {
    // What the next build weights itself with.
    let mut m = ProgressModel::new(81);
    m.tick(0);
    m.observe("▸ Welcome art");
    m.tick(10);
    m.observe("→ cargo build -p suisei-engine --release");
    m.tick(310);
    m.observe("→ swiftc → x");
    let measured = m.measured(910);

    assert!(!measured.secs.is_empty());
    let total: u64 = measured.secs.iter().map(|(_, s)| *s).sum();
    assert_eq!(total, 910, "the parts must add up to the whole");
    // The engine step ran from 10s to 310s.
    let engine = measured.secs.iter().find(|(i, _)| *i == 3).map(|(_, s)| *s);
    assert_eq!(engine, Some(300));
}

#[test]
fn measured_weights_replace_the_estimated_ones() {
    // A machine where Swift dominates should show the engine step finishing
    // sooner than the default weights say, because on THAT machine it does.
    let swift_heavy = Calibration {
        secs: vec![(3, 100), (4, 900)],
    };
    let mut estimated = ProgressModel::new(10);
    let mut measured = ProgressModel::with_calibration(10, swift_heavy);
    for m in [&mut estimated, &mut measured] {
        m.tick(0);
        m.observe("→ cargo build -p suisei-engine --release");
        for i in 0..10 {
            m.observe(&format!("Compiling c{i} v1.0"));
        }
    }
    assert!(
        measured.progress(60).fraction < estimated.progress(60).fraction,
        "measured {} vs estimated {}",
        measured.progress(60).fraction,
        estimated.progress(60).fraction
    );
}
