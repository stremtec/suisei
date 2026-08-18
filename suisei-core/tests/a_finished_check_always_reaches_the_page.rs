//! The Software Update page is PULLED, not pushed — and the pull has a gate.
//!
//! `refreshSoftwareUpdateIfNeeded` in the face reads the snapshot only when the
//! engine's update generation moves. The generation used to move when
//! `UpdateState::poll` returned a status MESSAGE, and those are not the same
//! event: "you are already up to date" is precisely the outcome with nothing to
//! print. So the one branch that produced no message also produced no refresh,
//! the page kept the snapshot it took when the check started, and
//! "Checking for Updates…" spun forever.
//!
//! It only bit users who were CURRENT, which is why it survived release — every
//! test machine had an update waiting, took the branch that did have something
//! to say, and refreshed. Observed on 0.1.1 immediately after a successful
//! update, which is the first moment the app had nothing left to fetch.
//!
//! The rule these tests hold: **every way a check can end moves the
//! generation** — found, not found, or the worker died without answering.
//!
//! ```text
//! cargo test -p suisei-core --test a_finished_check_always_reaches_the_page
//! ```

use suisei_core::update::{LatestRelease, UpdateState};

fn release(tag: &str) -> LatestRelease {
    LatestRelease {
        tag: tag.into(),
        sha: "2222222222222222222222222222222222222222".into(),
        notes: String::new(),
    }
}

#[test]
fn already_up_to_date_still_moves_the_generation() {
    // The regression, exactly. `poll` returns None here — there is nothing to
    // say — and that must NOT be read as "nothing happened".
    let mut u = UpdateState::new();
    u.deliver_check_for_test(None);
    let before = u.generation();

    let msg = u.poll();

    assert_eq!(msg, None, "an up-to-date check has nothing to print");
    assert_ne!(u.generation(), before, "…but the page still has to be told");
    assert!(!u.is_checking(), "the spinner stops");
}

#[test]
fn finding_an_update_moves_the_generation() {
    let mut u = UpdateState::new();
    u.deliver_check_for_test(Some(release("0.1.2")));
    let before = u.generation();

    u.poll();

    assert_ne!(u.generation(), before);
    assert_eq!(u.latest.as_deref(), Some("0.1.2"));
    assert!(!u.is_checking());
}

#[test]
fn finding_an_update_is_not_a_status_line() {
    // This returned `start_install`'s gate string — "This is not a valid
    // Suisei release." — so a check that SUCCEEDED printed a failure. The page
    // draws "Update Available" from the snapshot and offers a button; the
    // status line has no part in it.
    let mut u = UpdateState::new();
    u.deliver_check_for_test(Some(release("0.1.2")));

    assert_eq!(u.poll(), None);
}

#[test]
fn a_worker_that_dies_without_answering_still_stops_the_spinner() {
    // A panicked check thread drops its sender. Nothing to report, but a
    // spinner that never stops is a worse answer than the wrong one.
    let mut u = UpdateState::new();
    u.abandon_check_for_test();
    let before = u.generation();

    u.poll();

    assert_ne!(u.generation(), before);
    assert!(!u.is_checking());
}

#[test]
fn a_check_still_running_does_not_move_the_generation() {
    // The other half of the rule: an idle poll during a real check must not
    // churn the face's snapshot every frame.
    let mut u = UpdateState::new();
    let before = u.generation();

    for _ in 0..10 {
        assert_eq!(u.poll(), None);
    }

    assert_eq!(u.generation(), before, "an idle poll changes nothing");
}

#[test]
fn the_generation_moves_once_per_check_not_once_per_poll() {
    let mut u = UpdateState::new();
    u.deliver_check_for_test(None);

    u.poll();
    let after_check = u.generation();
    for _ in 0..10 {
        u.poll();
    }

    assert_eq!(u.generation(), after_check, "the drained check is done");
}
