//! Format on save, and the four ways it could eat your work.
//!
//! The formatter is a language server, so it answers when it feels like it.
//! That makes format-on-save an asynchronous save: ⌘S asks, and the write
//! happens later. Every risk in the feature is on the same axis — **the file
//! does not get written** — so every test here is about that.
//!
//!   · The server never answers → the file is written anyway, unformatted.
//!   · The server says "nothing to change" → that is an ANSWER, and the save
//!     goes through immediately rather than waiting out the timeout.
//!   · The user keeps typing while it thinks → the reply is a whole-buffer
//!     replacement describing a document that no longer exists, so it is
//!     dropped and the file is saved as typed.
//!   · The setting is off, or there is no server → the plain write, untouched.
//!
//! ```text
//! cargo test -p suisei-core --test a_save_is_never_lost_to_the_formatter
//! ```

use suisei_core::app::App;
use suisei_core::lsp::FileEdit;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("suisei_format_on_save");
    std::fs::create_dir_all(&dir).expect("temp dir");
    // One file per test: these run in parallel and a shared path is a race.
    dir.join(format!("{name}.rs"))
}

/// An app on a real file, with the setting on and a server pretending to run.
fn app_with(name: &str, text: &str, on: bool, server: bool) -> (App, std::path::PathBuf) {
    let path = scratch(name);
    std::fs::write(&path, text).expect("write");
    let mut app = App::open_file(path.to_str().unwrap());
    app.settings.draft.format_on_save = on;
    app.lsp.server_running = server;
    (app, path)
}

fn on_disk(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn with_the_setting_off_a_save_is_just_a_save() {
    let (mut app, path) = app_with("off", "fn  a(){}\n", false, true);
    app.buffer = suisei_core::buffer::Buffer::from_string("fn  b(){}\n");
    app.save_file_formatted();
    assert!(app.pending_save.is_none(), "nothing was held");
    assert_eq!(on_disk(&path), "fn  b(){}\n", "written, and not reformatted");
}

#[test]
fn with_no_server_running_a_save_is_just_a_save() {
    // The setting can be on in a file whose language has no server at all.
    // Holding the write for a formatter that does not exist would mean ⌘S did
    // nothing until the timeout, every time, in that file.
    let (mut app, path) = app_with("noserver", "x\n", true, false);
    app.buffer = suisei_core::buffer::Buffer::from_string("y\n");
    app.save_file_formatted();
    assert!(app.pending_save.is_none());
    assert_eq!(on_disk(&path), "y\n");
}

#[test]
fn a_formatted_save_holds_the_write_until_the_answer() {
    let (mut app, path) = app_with("hold", "fn  a(){}\n", true, true);
    app.buffer = suisei_core::buffer::Buffer::from_string("fn  b(){}\n");
    app.save_file_formatted();
    assert!(app.pending_save.is_some(), "the save is waiting");
    assert_eq!(on_disk(&path), "fn  a(){}\n", "not written yet");
}

#[test]
fn the_answer_is_applied_and_then_the_file_is_written() {
    let (mut app, path) = app_with("applied", "fn  a(){}\n", true, true);
    app.buffer = suisei_core::buffer::Buffer::from_string("fn  b(){}\n");
    app.save_file_formatted();

    // The server replies with the tidied whole buffer.
    app.lsp.formatting_answered = true;
    app.lsp.pending_edits = vec![FileEdit {
        path: path.display().to_string(),
        text: "fn b() {}\n".into(),
    }];
    app.poll_language_services();

    assert!(app.pending_save.is_none(), "the hold is released");
    assert_eq!(app.buffer.text(), "fn b() {}\n", "buffer formatted");
    assert_eq!(on_disk(&path), "fn b() {}\n", "and written formatted");
}

#[test]
fn nothing_to_change_is_an_answer_and_saves_at_once() {
    // The "already tidy" case. `pending_edits` stays EMPTY here, so a design
    // that waited for edits rather than for an answer would stall every save
    // of an already-formatted file until the timeout.
    let (mut app, path) = app_with("tidy", "fn a() {}\n", true, true);
    app.buffer = suisei_core::buffer::Buffer::from_string("fn b() {}\n");
    app.save_file_formatted();
    assert!(app.pending_save.is_some());

    app.lsp.formatting_answered = true;
    assert!(app.lsp.pending_edits.is_empty());
    app.poll_language_services();

    assert!(app.pending_save.is_none());
    assert_eq!(on_disk(&path), "fn b() {}\n");
}

#[test]
fn typing_while_it_thinks_saves_what_you_typed() {
    // The reply is a WHOLE-BUFFER replacement of the document as it was when
    // we asked. Applying it after the user has typed would put the file back
    // and take those keystrokes with it — silently, and on every save.
    let (mut app, path) = app_with("raced", "fn  a(){}\n", true, true);
    app.buffer = suisei_core::buffer::Buffer::from_string("fn  b(){}\n");
    app.save_file_formatted();

    // The user carries on typing.
    app.buffer = suisei_core::buffer::Buffer::from_string("fn  b(){}\nlet kept = 1;\n");

    // …and only now does the formatter answer, about the older document.
    app.lsp.formatting_answered = true;
    app.lsp.pending_edits = vec![FileEdit {
        path: path.display().to_string(),
        text: "fn b() {}\n".into(),
    }];
    app.poll_language_services();

    assert!(
        app.buffer.text().contains("let kept = 1;"),
        "the typing survived: {:?}",
        app.buffer.text()
    );
    assert!(on_disk(&path).contains("let kept = 1;"), "and was written");
    assert!(app.pending_save.is_none());
}

#[test]
fn a_server_that_never_answers_still_gets_the_file_written() {
    // The whole feature is worse than not having it if a wedged rust-analyzer
    // can hold a file hostage. Unformatted beats unsaved.
    let (mut app, path) = app_with("hung", "fn  a(){}\n", true, true);
    app.buffer = suisei_core::buffer::Buffer::from_string("fn  b(){}\n");
    app.save_file_formatted();
    assert_eq!(on_disk(&path), "fn  a(){}\n", "still held");

    // Wind the clock past the budget. Nothing ever replies.
    if let Some(p) = app.pending_save.as_mut() {
        p.asked_at = std::time::Instant::now() - std::time::Duration::from_millis(2_000);
    }
    app.poll_language_services();

    assert!(app.pending_save.is_none(), "the hold was let go");
    assert_eq!(on_disk(&path), "fn  b(){}\n", "written, unformatted");
}

#[test]
fn a_second_save_while_one_waits_is_the_same_save() {
    // Two asks would leave two replies for one write.
    let (mut app, _path) = app_with("double", "fn  a(){}\n", true, true);
    app.save_file_formatted();
    let first = app.pending_save.as_ref().map(|p| p.asked_at);
    app.save_file_formatted();
    let second = app.pending_save.as_ref().map(|p| p.asked_at);
    assert_eq!(first, second, "the hold was not restarted");
}

#[test]
fn it_is_off_until_someone_asks_for_it() {
    // A formatter rewrites the whole file. On by default would mean the first
    // save in a shared repository produces a diff nobody asked for.
    assert!(!suisei_core::config::Config::default().format_on_save);
}

#[test]
fn the_settings_row_is_wired_to_the_field_it_names() {
    // The likely bug in a new row is not that it fails to appear — it is that
    // it appears and drives a NEIGHBOURING field, which reads as the toggle
    // doing nothing. Set it through the panel and read it back through the
    // panel, which is the path Settings actually uses.
    use suisei_core::settings::{SettingRow, SettingsPage, SettingsPanel};
    let row = SettingRow::FormatOnSave;
    let mut panel = SettingsPanel::new();
    panel.page = SettingsPage::Setting;
    panel.selected = panel
        .setting_rows()
        .iter()
        .position(|r| *r == row)
        .expect("the row is not listed");
    let lsp_before = panel.draft.lsp_enabled;

    panel.set_value(1);
    assert!(panel.draft.format_on_save, "the toggle did not reach the field");
    assert_eq!(row.value_index(&panel.draft), 1, "and does not read back");
    assert_eq!(panel.draft.lsp_enabled, lsp_before, "and touched nothing else");

    panel.set_value(0);
    assert!(!panel.draft.format_on_save);
    assert_eq!(row.value_index(&panel.draft), 0);
}
