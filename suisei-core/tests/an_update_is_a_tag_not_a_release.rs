//! Which commit is the next version, and how we ask.
//!
//! The old check asked `api.github.com/repos/…/releases/latest`. Measured while
//! designing the replacement, on a developer's own machine: **HTTP 403, `0/60`
//! remaining.** Unauthenticated GitHub REST is sixty requests an hour *per IP*,
//! and every other tool sharing that address spends from the same bucket. On an
//! office, campus or CGNAT connection the update check is broken most of the
//! time — silently, because a failed check is indistinguishable from "you are
//! up to date". `git ls-remote` has no such limit, over the same TLS to the
//! same host.
//!
//! It also asks a better question. A GitHub *Release* is an artifact to
//! download; a **tag is a name for a commit**, which is what an update that
//! builds from source actually needs.
//!
//! ```text
//! cargo test -p suisei-core --test an_update_is_a_tag_not_a_release
//! ```

use suisei_core::update;

/// Real `git ls-remote --tags` output shape, including the two-line form.
const LS_REMOTE: &str = "\
4f9c8cbf2386b5e35c5ba754b705c383c5f4b4cc\trefs/tags/v0.1.0
051478957371ee0084a7c0913941d2a8c4757bb9\trefs/tags/v0.1.0^{}
0071a0c49a7e6958ea6c493098d2c6a6040447b3\trefs/tags/v0.2.0
eeb90cda1969383f56a2637cbd3037bdf598841c\trefs/tags/v0.2.0^{}
";

#[test]
fn the_newest_tag_wins() {
    let r = update::parse_ls_remote_for_test(LS_REMOTE).expect("a release");
    assert_eq!(r.tag, "0.2.0");
}

#[test]
fn an_annotated_tag_resolves_to_its_commit_not_its_tag_object() {
    // `refs/tags/X` is the tag OBJECT; `refs/tags/X^{}` is the commit it points
    // at. Cloning the first checks out nothing, because it is not a commit —
    // and the failure arrives twenty minutes into a build, not here.
    let r = update::parse_ls_remote_for_test(LS_REMOTE).expect("a release");
    assert_eq!(
        r.sha, "eeb90cda1969383f56a2637cbd3037bdf598841c",
        "took the tag object instead of the commit"
    );
}

#[test]
fn a_lightweight_tag_keeps_its_own_sha() {
    // No `^{}` line at all — the single line IS the commit.
    let text = "abc1230000000000000000000000000000000000\trefs/tags/v0.3.0\n";
    let r = update::parse_ls_remote_for_test(text).expect("a release");
    assert_eq!(r.tag, "0.3.0");
    assert_eq!(r.sha, "abc1230000000000000000000000000000000000");
}

#[test]
fn versions_compare_as_numbers_not_as_text() {
    // Refs come back in LEXICAL order, where v0.10.0 sorts before v0.9.0.
    // Taking the last line — or comparing as strings — walks the version
    // backwards on exactly the release where it matters.
    let text = "\
1111111111111111111111111111111111111111\trefs/tags/v0.9.0
2222222222222222222222222222222222222222\trefs/tags/v0.10.0
3333333333333333333333333333333333333333\trefs/tags/v0.11.0
4444444444444444444444444444444444444444\trefs/tags/v0.9.9
";
    let r = update::parse_ls_remote_for_test(text).expect("a release");
    assert_eq!(r.tag, "0.11.0");
    assert_eq!(r.sha, "3333333333333333333333333333333333333333");
}

#[test]
fn the_leftover_xei_tags_are_not_suisei_releases() {
    // The pre-fork repository's tags (3.0.10, …) are visible to `ls-remote`
    // and are numerically HIGHER than anything Suisei has released. Without
    // the guard, every install would offer an "upgrade" to someone else's
    // project — and `ls-remote` sees every tag, where `releases/latest` saw
    // one, so this matters more now than it did.
    let text = "\
1111111111111111111111111111111111111111\trefs/tags/v0.2.0
9999999999999999999999999999999999999999\trefs/tags/v3.0.10
";
    let r = update::parse_ls_remote_for_test(text).expect("a release");
    assert_eq!(r.tag, "0.2.0", "3.0.10 is an xei tag");
}

#[test]
fn a_repository_with_no_tags_yet_offers_nothing() {
    // Which is the state the repository is actually in today: `ls-remote`
    // exits 0 and prints nothing. That has to read as "no update", not as an
    // error and not as a panic.
    assert!(update::parse_ls_remote_for_test("").is_none());
}

#[test]
fn branches_and_other_refs_are_ignored() {
    let text = "\
1111111111111111111111111111111111111111\trefs/heads/main
2222222222222222222222222222222222222222\trefs/pull/12/head
3333333333333333333333333333333333333333\trefs/tags/v0.4.0
";
    let r = update::parse_ls_remote_for_test(text).expect("a release");
    assert_eq!(r.tag, "0.4.0");
}

#[test]
fn a_tag_that_is_not_a_version_is_skipped() {
    let text = "\
1111111111111111111111111111111111111111\trefs/tags/nightly
2222222222222222222222222222222222222222\trefs/tags/v0.5.0
";
    let r = update::parse_ls_remote_for_test(text).expect("a release");
    assert_eq!(r.tag, "0.5.0");
}

#[test]
fn a_repository_whose_only_tag_is_not_a_version_offers_nothing() {
    // The version of the test above that actually pins the behaviour. With
    // `v0.5.0` present, `nightly` merely LOSES the comparison — it parses to
    // version `[0]` — so the assertion above passes whether or not it is
    // filtered. Alone, it must read as no release rather than as the newest.
    let text = "\
1111111111111111111111111111111111111111\trefs/tags/nightly
2222222222222222222222222222222222222222\trefs/tags/latest
3333333333333333333333333333333333333333\trefs/tags/ci-run-4
";
    assert!(update::parse_ls_remote_for_test(text).is_none());
}

#[test]
fn a_third_tag_supersedes_the_second_and_the_first() {
    // The shipped situation, spelled out with the real tag names rather than
    // stand-ins: v0.1.0 and v0.1.1 are published, v0.1.2 lands beside them.
    // Every tag stays on the remote forever — publishing a new one does not
    // retire the old ones — so "the newest" has to be DECIDED on every check
    // and not inferred from the order the remote happens to list them in.
    let text = "\
89a6caf783c6de4836109b69682044b61239de94\trefs/tags/v0.1.0
f93cd9c5b39dad482c71c02f31613c98aff96492\trefs/tags/v0.1.0^{}
dd5d338a293076633f1b4c6262787d68eff12817\trefs/tags/v0.1.1
cecb7969f276803fdf086b92c6829073a8e50f6f\trefs/tags/v0.1.1^{}
1111111111111111111111111111111111111111\trefs/tags/v0.1.2
2222222222222222222222222222222222222222\trefs/tags/v0.1.2^{}
";
    let r = update::parse_ls_remote_for_test(text).expect("a release");
    assert_eq!(r.tag, "0.1.2");
    // And it builds v0.1.2's COMMIT, not its tag object.
    assert_eq!(r.sha, "2222222222222222222222222222222222222222");
}

#[test]
fn the_newest_tag_wins_however_the_remote_orders_them() {
    // `git ls-remote` sorts by refname, which is alphabetical — so v0.1.10
    // arrives BEFORE v0.1.9 and the last line read is not the answer.
    let text = "\
1111111111111111111111111111111111111111\trefs/tags/v0.1.10
2222222222222222222222222222222222222222\trefs/tags/v0.1.9
";
    let r = update::parse_ls_remote_for_test(text).expect("a release");
    assert_eq!(r.tag, "0.1.10");
}

#[test]
fn the_repository_is_the_one_the_licence_names() {
    // A self-updater pointed at the wrong repository is a supply chain, so
    // this is worth an assertion rather than a reading.
    assert_eq!(update::REPO_URL, "https://github.com/stremtec/suisei");
}
