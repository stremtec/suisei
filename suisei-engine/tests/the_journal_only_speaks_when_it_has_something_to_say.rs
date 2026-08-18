//! Crash recovery, and the two ways it can lie.
//!
//! Reported: "suisei 실행할 때 gone.txt 란게 엄청나게 언세이브드되었다고 뜸".
//! Forty-six recovery entries at every launch, all of them for
//! `$TMPDIR/suisei_tick_dirty_del_<pid>/gone.txt` — a file from ONE engine
//! test, which types a character into a temp file and then deletes it. The
//! journal was on by default in `Engine::new()`, so every test run in this
//! workspace wrote into the developer's real `~/.suisei/journal`, under a new
//! pid, so a new hash, so a new file. Nothing ever removed them.
//!
//! The machinery that says "you have unsaved work" is the one thing in an
//! editor that must never cry wolf: the cost is not the noise, it is that the
//! sheet gets dismissed unread, and one day it will have something real in it.
//!
//! ```text
//! cargo test -p suisei-engine --test the_journal_only_speaks_when_it_has_something_to_say
//! ```

use std::path::PathBuf;
use suisei_engine::journal::Journal;
use suisei_engine::runtime::Engine;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("suisei_journal_honesty/{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

/// Write a WAL by hand, with a timestamp of our choosing. `age_secs` back from
/// now, because the only rule here that involves time needs to be reachable
/// without waiting a week.
fn plant(wal_dir: &PathBuf, file_path: &str, text: &str, age_secs: u64) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in file_path.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    let body = format!(
        "SUISEI-WAL v1\npath: {file_path}\ncursor_row: 0\ncursor_col: 0\n\
         scroll: 0\ntimestamp: {}\n---\n{text}",
        now.saturating_sub(age_secs)
    );
    std::fs::write(wal_dir.join(format!("{hash:016x}.wal")), body).expect("plant");
}

fn wals(wal_dir: &PathBuf) -> usize {
    std::fs::read_dir(wal_dir)
        .map(|d| {
            d.flatten()
                .filter(|e| e.path().extension().map(|x| x == "wal").unwrap_or(false))
                .count()
        })
        .unwrap_or(0)
}

/// The bug itself, at its root. A bare engine is what every test in this
/// workspace builds, and building one must not be enough to write into
/// somebody's home directory.
#[test]
fn a_bare_engine_records_nothing_anywhere() {
    let engine = Engine::new();
    assert!(!engine.journal.is_recording());
    assert_eq!(engine.journal.dir(), None, "no directory, so no file");
    assert_eq!(engine.journal.recovery_count(), 0, "and nothing to offer");
}

/// A file deleted out from under unsaved edits is a case this editor supports
/// — the tab shows it, the buffer stays dirty — so it is exactly the case the
/// journal exists for. The folder is still there, so the text has somewhere to
/// go. This is the entry that MUST survive every rule below it.
#[test]
fn work_whose_file_was_deleted_is_still_offered() {
    let wal = dir("deleted/wal");
    let work = dir("deleted/work");
    let gone = work.join("gone.txt");
    plant(&wal, gone.to_str().unwrap(), "hello, unsaved\n", 0);

    let journal = Journal::with_dir(wal.clone());
    assert_eq!(journal.recovery_count(), 1);
    assert_eq!(journal.recovery_entry(0).unwrap().text, "hello, unsaved\n");
    assert_eq!(wals(&wal), 1, "and the file is left where it is");
}

/// Nowhere to put it. The folder is gone, not just the file — which is what
/// the reported forty-six were, temp directories wiped after each test run.
#[test]
fn work_with_nowhere_to_go_is_not_offered_and_ages_out() {
    let wal = dir("unreachable");
    let vanished = std::env::temp_dir().join("suisei_journal_honesty/never_existed_at_all");
    let _ = std::fs::remove_dir_all(&vanished);
    let target = vanished.join("gone.txt");

    plant(&wal, target.to_str().unwrap(), "!hello\n", 60);
    let journal = Journal::with_dir(wal.clone());
    assert_eq!(journal.recovery_count(), 0, "not offered");
    assert_eq!(
        wals(&wal),
        1,
        "but kept — an unmounted volume looks exactly like this, and it comes back"
    );
    drop(journal);

    // A week later the crash it belongs to is over.
    let old = dir("unreachable_old");
    plant(&old, target.to_str().unwrap(), "!hello\n", 8 * 24 * 60 * 60);
    let journal = Journal::with_dir(old.clone());
    assert_eq!(journal.recovery_count(), 0);
    assert_eq!(wals(&old), 0, "and now it is swept up");
}

/// Nothing to recover: the file on disk already says what the journal was
/// holding. A save that landed while its WAL delete did not, or another editor
/// writing the same text.
#[test]
fn work_that_is_already_on_disk_is_not_a_recovery() {
    let wal = dir("same/wal");
    let work = dir("same/work");
    let f = work.join("saved.rs");
    std::fs::write(&f, "fn main() {}\n").expect("write");
    plant(&wal, f.to_str().unwrap(), "fn main() {}\n", 0);

    let journal = Journal::with_dir(wal.clone());
    assert_eq!(journal.recovery_count(), 0);
    assert_eq!(wals(&wal), 0, "rubbish, so it goes");

    // One character apart is a recovery.
    let wal2 = dir("same/wal2");
    plant(&wal2, f.to_str().unwrap(), "fn main() { 1 }\n", 0);
    assert_eq!(Journal::with_dir(wal2).recovery_count(), 1);
}

/// The document comes back as the document. `lines().join("\n")` rebuilt it
/// without its final newline — and every Rust, Go and C file on disk ends with
/// one — so recovering a file used to hand back something subtly different
/// from what was lost, showing as a diff on its last line.
#[test]
fn a_recovered_document_is_the_document() {
    let wal = dir("verbatim/wal");
    let work = dir("verbatim/work");
    let f = work.join("x.rs");
    std::fs::write(&f, "old\n").expect("write");

    let mut journal = Journal::with_dir(wal.clone());
    let text = "fn main() {\n\n    let x = 1;\n}\n".to_string();
    journal.on_tick(f.to_str().unwrap(), || text.clone(), 1, 0, 0, 0, true);
    journal.on_tick(f.to_str().unwrap(), || text.clone(), 500, 0, 0, 0, true);
    journal.drain();
    drop(journal);

    let back = Journal::with_dir(wal);
    assert_eq!(back.recovery_count(), 1);
    assert_eq!(
        back.recovery_entry(0).unwrap().text,
        text,
        "byte for byte, including the blank line and the last newline"
    );
}
