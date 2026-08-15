//! Shadow WAL journal — write, recover, accept/discard cycle tests.
//!
//! ```text
//! cargo test -p suisei-engine --test journal_wal
//! ```

use suisei_engine::journal::Journal;

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("suisei_wal_test_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn wal_write_and_scan() {
    let dir = tmp_dir("write");
    let mut journal = Journal::with_dir(dir.clone());

    let text = "fn main() {\n    println!(\"hello\");\n}\n".repeat(200);
    let path = "/tmp/test_file.rs";

    // First tick: establishes baseline version.
    journal.on_tick(path, || text.clone(), 1, 5, 3, 10, true);
    // Second tick: delta = 99, pending_bytes = 99*64 = 6336 > 4096 → flush.
    journal.on_tick(path, || text.clone(), 100, 5, 3, 10, true);
    // The write left the tick — the fsync happens on the WAL thread. Nothing
    // on the tick path waits for it; a test that looks at the directory does.
    journal.drain();

    // Find the WAL file.
    let wal_files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "wal").unwrap_or(false))
        .collect();
    assert_eq!(wal_files.len(), 1, "exactly one WAL file after flush");

    // Verify WAL format.
    let content = std::fs::read_to_string(wal_files[0].path()).unwrap();
    assert!(content.starts_with("SUISEI-WAL v1\n"), "WAL magic header");
    assert!(
        content.contains(&format!("path: {path}\n")),
        "WAL path field"
    );
    assert!(content.contains("cursor_row: 5\n"), "WAL cursor_row");
    assert!(content.contains("cursor_col: 3\n"), "WAL cursor_col");
    assert!(content.contains("scroll: 10\n"), "WAL scroll");
    assert!(content.contains("---\n"), "WAL separator");
    assert!(content.contains("fn main()"), "WAL contains buffer text");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wal_saved_removes_entry() {
    let dir = tmp_dir("saved");
    let mut journal = Journal::with_dir(dir.clone());

    let path = "/tmp/saved_file.rs";
    let text = "saved content";

    // Force a flush (size threshold).
    journal.on_tick(path, || text.to_string(), 1, 0, 0, 0, true);
    journal.on_tick(path, || text.to_string(), 100, 0, 0, 0, true);

    // Save: should remove the journal entry.
    journal.on_saved(path);
    journal.drain();

    // WAL file should be gone.
    let wal_count = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "wal").unwrap_or(false))
        .count();
    assert_eq!(wal_count, 0, "no WAL files after save");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recovery_scan_and_accept() {
    let dir = tmp_dir("recover");
    let path = "/tmp/recover_me.rs";
    let text = "unsaved work that was lost in a crash";

    // Write a WAL entry.
    {
        let mut journal = Journal::with_dir(dir.clone());
        journal.on_tick(path, || text.to_string(), 1, 10, 5, 20, true);
        journal.on_tick(path, || text.to_string(), 200, 10, 5, 20, true);
    }

    // Simulate restart: fresh journal scans the same directory.
    let journal2 = Journal::with_dir(dir.clone());
    let count = journal2.recovery_count();
    assert!(count >= 1, "at least one recovery entry after crash");

    // Find our file.
    let mut found = false;
    for i in 0..count {
        if let Some(entry) = journal2.recovery_entry(i) {
            if entry.file_path == path {
                assert_eq!(entry.cursor_row, 10);
                assert_eq!(entry.cursor_col, 5);
                assert_eq!(entry.scroll, 20);
                assert!(entry.text.contains("unsaved work"));
                found = true;

                // Accept: returns the entry, deletes the WAL file.
                let mut journal3 = journal2;
                let accepted = journal3.accept_recovery(i);
                assert!(accepted.is_some(), "accept returns the entry");
                assert_eq!(accepted.unwrap().file_path, path);
                assert_eq!(journal3.recovery_count(), count - 1, "count decreases");
                break;
            }
        }
    }
    assert!(found, "recovery entry for {path} found");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recovery_discard_removes_wal() {
    let dir = tmp_dir("discard");
    let path = "/tmp/discard_me.rs";

    // Write a WAL entry.
    {
        let mut journal = Journal::with_dir(dir.clone());
        journal.on_tick(path, || "discarded work".to_string(), 1, 0, 0, 0, true);
        journal.on_tick(path, || "discarded work".to_string(), 100, 0, 0, 0, true);
    }

    let mut journal2 = Journal::with_dir(dir.clone());
    let count = journal2.recovery_count();
    assert!(count >= 1);

    // Discard the first entry.
    journal2.discard_recovery(0);
    assert_eq!(journal2.recovery_count(), count - 1);

    // WAL file should be gone from disk.
    let wal_count = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "wal").unwrap_or(false))
        .count();
    assert_eq!(wal_count, 0, "no WAL files after discard");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clean_buffer_not_journaled() {
    let dir = tmp_dir("clean");
    let mut journal = Journal::with_dir(dir.clone());

    // dirty=false → no WAL write.
    journal.on_tick(
        "/tmp/clean.rs",
        || "clean text".to_string(),
        1,
        0,
        0,
        0,
        false,
    );
    journal.drain();

    let wal_count = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "wal").unwrap_or(false))
        .count();
    assert_eq!(wal_count, 0, "clean buffer not journaled");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn untitled_not_journaled() {
    let dir = tmp_dir("untitled");
    let mut journal = Journal::with_dir(dir.clone());

    // Empty path → no WAL write.
    journal.on_tick("", || "unsaved text".to_string(), 1, 0, 0, 0, true);
    journal.drain();

    let wal_count = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "wal").unwrap_or(false))
        .count();
    assert_eq!(wal_count, 0, "untitled buffer not journaled");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Saving cancels a flush that has not reached the disk.
///
/// The risk the writer thread introduces. Deleting the WAL on the tick while a
/// write for the same file was still queued would let that write land
/// afterwards, and the next launch would offer to recover a file the user had
/// already saved. Both go through the queue, where a job for a path replaces
/// the pending one — so the save wins by construction rather than by timing.
#[test]
fn a_save_cancels_a_flush_that_has_not_landed() {
    let dir = tmp_dir("cancel");
    let mut journal = Journal::with_dir(dir.clone());
    let path = "/tmp/raced_file.rs";
    // Big enough that the write is not instantaneous.
    let text = "fn main() { let x = 1; }\n".repeat(20_000);

    journal.on_tick(path, || text.clone(), 1, 0, 0, 0, true);
    journal.on_tick(path, || text.clone(), 100, 0, 0, 0, true);
    // No drain: the write is deliberately still in the queue.
    journal.on_saved(path);
    journal.drain();

    let wal_count = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "wal").unwrap_or(false))
        .count();
    assert_eq!(wal_count, 0, "the save must win over the queued flush");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Quitting does not throw away a snapshot the tick already handed over.
///
/// Closing the queue drains it before the writer exits, and `Drop` joins. A
/// dirty buffer at quit is exactly when this matters.
#[test]
fn dropping_the_journal_finishes_what_was_queued() {
    let dir = tmp_dir("shutdown");
    let path = "/tmp/quit_while_dirty.rs";
    {
        let mut journal = Journal::with_dir(dir.clone());
        journal.on_tick(path, || "work in progress".to_string(), 1, 0, 0, 0, true);
        journal.on_tick(path, || "work in progress".to_string(), 100, 0, 0, 0, true);
        // No drain — the drop has to do it.
    }

    let recovered = Journal::with_dir(dir.clone());
    assert!(
        recovered
            .pending_recovery()
            .iter()
            .any(|e| e.file_path == path && e.text.contains("work in progress")),
        "a queued snapshot survives the quit"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The journal runs on the 20 Hz tick but flushes at most every 250 ms, and
/// only while dirty. Building the document for the ticks in between taxed every
/// session: 0.24 ms per tick on a 60,000-line file, thrown away immediately.
#[test]
fn a_tick_that_will_not_flush_never_builds_the_document() {
    let dir = tmp_dir("lazy");
    let mut journal = Journal::with_dir(dir);
    let mut builds = 0usize;

    journal.on_tick(
        "/tmp/lazy.rs",
        || {
            builds += 1;
            "text".to_string()
        },
        1,
        0,
        0,
        0,
        false, // clean
    );
    assert_eq!(
        builds, 0,
        "a clean buffer must not pay for a document build"
    );

    // Dirty, but the first tick only establishes the version baseline and the
    // 250 ms debounce has not elapsed — still nothing to write.
    journal.on_tick(
        "/tmp/lazy.rs",
        || {
            builds += 1;
            "text".to_string()
        },
        1,
        0,
        0,
        0,
        true,
    );
    assert_eq!(
        builds, 0,
        "a tick inside the debounce must not build it either"
    );
}
