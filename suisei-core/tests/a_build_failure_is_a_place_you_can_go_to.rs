//! Run a command, and land where it complained.
//!
//! feature.txt P2, the half `dap.rs` left behind. The debugger has always been
//! able to run `cargo build`; when the build failed it wrote one sentence into
//! the panel — the first `error:` line and its arrow, joined — and that
//! sentence names a file and a line that there was no way to go to. The other
//! twenty errors were dropped on the floor, and there was no way at all to run
//! `cargo test` and read what it said.
//!
//! What is asserted here is the whole claim: a process runs, its output is
//! kept, its complaints become diagnostics on the file they are about, and the
//! caret can be put on one.
//!
//! ```text
//! cargo test -p suisei-core --test a_build_failure_is_a_place_you_can_go_to
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use suisei_core::app::App;
use suisei_core::build::{Build, BuildKind, BuildState, Plan};

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("suisei_build/{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

/// A plan that runs a shell line, so the test is about the runner rather than
/// about whichever compilers happen to be installed on the machine.
fn shell(cwd: &PathBuf, script: &str) -> Plan {
    Plan {
        kind: BuildKind::Build,
        program: "sh".into(),
        args: vec!["-c".into(), script.into()],
        cwd: cwd.clone(),
        label: "sh".into(),
        json: false,
    }
}

fn run_to_completion(b: &mut Build, plan: &Plan) {
    b.start(plan).expect("spawned");
    let deadline = Instant::now() + Duration::from_secs(20);
    while b.state.is_running() && Instant::now() < deadline {
        b.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!b.state.is_running(), "the run ended");
}

#[test]
fn a_failing_command_is_read_to_the_end_and_then_reported_failed() {
    let d = dir("stream");
    let mut b = Build::default();
    run_to_completion(
        &mut b,
        &shell(
            &d,
            "echo out-one; echo err-one 1>&2; echo 'src/a.c:12:7: error: boom'; exit 3",
        ),
    );

    assert_eq!(b.state, BuildState::Failed);
    assert_eq!(b.exit, Some(3));
    assert!(b.took.is_some(), "how long it took is part of the answer");
    // Both pipes, and the command itself first so the console reads like one.
    let console: Vec<&str> = b.output.iter().map(|l| l.text.as_str()).collect();
    assert!(console[0].starts_with("$ sh -c"), "{console:?}");
    assert!(console.contains(&"out-one"), "{console:?}");
    assert!(console.contains(&"err-one"), "stderr too — {console:?}");
    assert!(
        console.last().unwrap().starts_with("· exit 3"),
        "{console:?}"
    );

    assert_eq!(b.problems.len(), 1);
    assert_eq!(b.error_count(), 1);
    let p = &b.problems[0];
    assert_eq!(p.path, d.join("src/a.c").display().to_string(), "absolute");
    assert_eq!((p.row, p.col), (11, 6), "0-based, both of them");
}

#[test]
fn a_command_that_succeeds_says_so_and_complains_about_nothing() {
    let d = dir("ok");
    let mut b = Build::default();
    run_to_completion(&mut b, &shell(&d, "echo fine"));
    assert_eq!(b.state, BuildState::Ok);
    assert_eq!(b.exit, Some(0));
    assert!(b.problems.is_empty());
    assert!(b.summary().contains("succeeded"), "{}", b.summary());
}

/// Pressing Run twice must run it again NOW. A second `cargo build` racing the
/// first over one `target/` directory is not something anyone asked for.
#[test]
fn a_second_run_replaces_the_first_rather_than_queueing_behind_it() {
    let d = dir("replace");
    let mut b = Build::default();
    b.start(&shell(&d, "sleep 30")).expect("spawned");
    assert!(b.state.is_running());

    run_to_completion(&mut b, &shell(&d, "echo second"));
    assert_eq!(b.exit, Some(0));
    let console: Vec<&str> = b.output.iter().map(|l| l.text.as_str()).collect();
    assert!(console.contains(&"second"), "{console:?}");
    assert!(
        !console.iter().any(|l| l.contains("sleep")),
        "the first run's console went with it — {console:?}"
    );
}

#[test]
fn stopping_kills_it_and_says_that_is_what_happened() {
    let d = dir("stop");
    let mut b = Build::default();
    b.start(&shell(&d, "sleep 30")).expect("spawned");
    b.stop();
    assert_eq!(b.state, BuildState::Failed);
    assert!(
        b.output.iter().any(|l| l.text.contains("stopped")),
        "{:?}",
        b.output
    );
    // And a stop with nothing running is not an error.
    b.stop();
}

/// The point of the whole feature: a compile error is a diagnostic, drawn by
/// the same code that draws the language server's, listed in the same list.
#[test]
fn a_build_problem_becomes_a_diagnostic_on_the_file_it_is_about() {
    let d = dir("diag");
    let a = d.join("a.rs");
    let b_file = d.join("b.rs");
    std::fs::write(&a, "fn main() {\n    let x = 1;\n}\n").expect("write");
    std::fs::write(&b_file, "fn other() {}\n").expect("write");

    let mut app = App::open_file(a.to_str().unwrap());
    assert!(!app.has_diagnostics(), "nothing has complained yet");

    run_to_completion(
        &mut app.build,
        &shell(
            &d,
            &format!(
                "echo '{}:2:9: error: unused'; echo '{}:1:4: warning: idle'; exit 1",
                a.display(),
                b_file.display()
            ),
        ),
    );
    app.sync_build_diagnostics();

    assert_eq!(app.build.problems.len(), 2, "both files complained");
    assert_eq!(
        app.diagnostic_count(),
        1,
        "but only this file's is on this screen"
    );
    let d0 = app.diagnostics().next().expect("one");
    assert_eq!((d0.row, d0.col_start), (1, 8));
    assert_eq!(app.diagnostics_for_row(1).count(), 1);
    assert_eq!(app.diagnostics_for_row(0).count(), 0, "not this row");

    // Walking to the other file changes the answer, and nothing had to tell
    // the diagnostics that — the file is part of what they are derived from.
    app.open_text_tab(b_file.to_str().unwrap());
    app.sync_build_diagnostics();
    assert_eq!(app.diagnostic_count(), 1);
    let d1 = app.diagnostics().next().expect("one");
    assert!(d1.message.contains("idle"), "{}", d1.message);
}

#[test]
fn going_to_a_problem_opens_its_file_and_puts_the_caret_on_it() {
    let d = dir("goto");
    let one = d.join("one.rs");
    let two = d.join("two.rs");
    std::fs::write(&one, "fn one() {}\n").expect("write");
    std::fs::write(&two, "fn two() {\n    let long_name = 1;\n}\n").expect("write");

    let mut app = App::open_file(one.to_str().unwrap());
    run_to_completion(
        &mut app.build,
        &shell(
            &d,
            &format!("echo '{}:2:9: error: unused'; exit 1", two.display()),
        ),
    );

    app.goto_build_problem(0);
    assert_eq!(
        app.filename.as_deref().map(|p| p.display().to_string()),
        Some(two.display().to_string()),
        "the other file was opened"
    );
    assert_eq!(app.buffer.cursor.row, 1);
    assert_eq!(app.buffer.cursor.col, 8);
    assert!(app.message.contains("unused"), "{}", app.message);

    // A problem with nowhere to go does not jump anywhere and says why.
    app.build.problems[0].path.clear();
    let before = app.buffer.cursor;
    app.goto_build_problem(0);
    assert_eq!(app.buffer.cursor, before);
    // And an index that does not exist is not a panic.
    app.goto_build_problem(99);
}

/// The project decides. `plan` guesses from the manifest, and the guess is
/// right for one target and no ceremony; a repository that means something
/// else has already decided it somewhere, and this is where it says so.
#[test]
fn a_project_can_name_its_own_run_command() {
    let d = dir("custom");
    std::fs::write(d.join("Cargo.toml"), "[package]\nname=\"x\"\n").expect("write");
    let mut p = suisei_core::project::ensure(&d).expect("project");
    p.settings
        .commands
        .insert("run".into(), "just serve --port 3000".into());
    suisei_core::project::write(&d, &p).expect("written");

    let back = suisei_core::project::read(&d).expect("read");
    assert_eq!(
        back.settings.commands.get("run").map(String::as_str),
        Some("just serve --port 3000"),
        "it survives the file"
    );
    // A key with no button is dropped rather than silently doing nothing.
    std::fs::write(
        d.join(suisei_core::project::MARKER),
        "{\"schema\":1,\"name\":\"x\",\"settings\":{\"commands\":{\"deploy\":\"rm -rf /\"}}}",
    )
    .expect("write");
    assert!(
        suisei_core::project::read(&d)
            .unwrap()
            .settings
            .commands
            .is_empty()
    );
}
