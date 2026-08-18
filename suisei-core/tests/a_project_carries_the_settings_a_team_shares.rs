//! `project.suiseiprj` — what a repository decides, as opposed to a person.
//!
//! feature.txt ☐22 (the screen) and P3 (per-project settings): the same file
//! and the same writer, so they are one feature. A settings screen with
//! nothing on it is not worth opening.
//!
//! The file is committed to a repository and read by a team, so its shape is a
//! promise: written by hand rather than by a serializer, stable key order, and
//! nothing written that was not decided.
//!
//! ```text
//! cargo test -p suisei-core --test a_project_carries_the_settings_a_team_shares
//! ```

use std::path::PathBuf;
use suisei_core::project::{self, Project, ProjectSettings};

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("suisei_project_settings/{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

fn text(d: &PathBuf) -> String {
    std::fs::read_to_string(d.join(project::MARKER)).expect("marker")
}

/// A project with no opinion writes no opinion.
///
/// An empty `"settings": {}` is a line of noise in a file a team reads — and
/// worse, it looks like a decision.
#[test]
fn a_project_that_decided_nothing_says_nothing() {
    let d = dir("silent");
    project::ensure(&d).expect("created");
    let on_disk = text(&d);
    assert!(!on_disk.contains("settings"), "{on_disk}");
    assert_eq!(project::read(&d).unwrap().settings, ProjectSettings::default());
}

/// What is decided is written, and comes back exactly.
#[test]
fn an_indent_width_survives_the_round_trip() {
    let d = dir("indent");
    let mut p = project::ensure(&d).expect("created");
    p.settings.tab_width = Some(2);
    project::write(&d, &p).expect("written");

    assert!(text(&d).contains("\"tab_width\": 2"));
    assert_eq!(project::read(&d).unwrap().settings.tab_width, Some(2));

    // And clearing it takes the whole section with it: absence is how a
    // project says "no opinion", so it has to be reachable from the screen.
    p.settings.tab_width = None;
    project::write(&d, &p).expect("written");
    assert!(!text(&d).contains("settings"));
    assert_eq!(project::read(&d).unwrap().settings.tab_width, None);
}

#[test]
fn language_servers_are_written_in_a_stable_order() {
    let d = dir("lsp");
    let mut p = project::ensure(&d).expect("created");
    p.settings
        .lsp_servers
        .insert("rust".into(), "rust-analyzer --log-file /tmp/ra".into());
    p.settings
        .lsp_servers
        .insert("python".into(), "pyright-langserver --stdio".into());
    project::write(&d, &p).expect("written");

    let on_disk = text(&d);
    let python = on_disk.find("\"python\"").expect("python is there");
    let rust = on_disk.find("\"rust\"").expect("rust is there");
    assert!(python < rust, "sorted, so a diff shows only what changed");

    let back = project::read(&d).unwrap();
    assert_eq!(back.settings.lsp_servers.len(), 2);
    assert_eq!(
        back.settings.lsp_servers.get("rust").map(String::as_str),
        Some("rust-analyzer --log-file /tmp/ra")
    );
}

/// The file is meant to be edited by hand — that is the whole point of the
/// escape hatch under the screen — so a number that would break something far
/// away is clamped where it is read.
#[test]
fn a_hand_written_nonsense_width_is_clamped_rather_than_obeyed() {
    let d = dir("hand");
    std::fs::write(
        d.join(project::MARKER),
        "{\"schema\":1,\"project_id\":\"x\",\"name\":\"n\",\"settings\":{\"tab_width\":0}}",
    )
    .expect("write");
    assert_eq!(project::read(&d).unwrap().settings.tab_width, Some(1));

    std::fs::write(
        d.join(project::MARKER),
        "{\"schema\":1,\"project_id\":\"x\",\"name\":\"n\",\"settings\":{\"tab_width\":900}}",
    )
    .expect("write");
    assert_eq!(project::read(&d).unwrap().settings.tab_width, Some(16));
}

/// A file that is not JSON at all still yields a project rather than nothing:
/// the folder IS a project — the marker is there — and losing its identity
/// because someone left a stray brace would be the worst possible answer.
#[test]
fn a_broken_marker_still_names_the_project() {
    let d = dir("broken");
    std::fs::write(d.join(project::MARKER), "{ not json at all").expect("write");
    let p = project::read(&d).expect("still a project");
    assert_eq!(p.name, "broken");
    assert_eq!(p.settings, ProjectSettings::default());
}

/// The whole file, once, as a human will see it in a diff.
#[test]
fn the_written_file_is_the_shape_a_human_reads() {
    let d = dir("shape");
    let mut p = Project {
        schema: 1,
        project_id: "abc123".into(),
        name: "Suisei".into(),
        settings: ProjectSettings::default(),
    };
    p.settings.tab_width = Some(4);
    p.settings.lsp_servers.insert("rust".into(), "rust-analyzer".into());
    project::write(&d, &p).expect("written");

    assert_eq!(
        text(&d),
        "{\n  \"schema\": 1,\n  \"project_id\": \"abc123\",\n  \"name\": \"Suisei\",\n  \
         \"settings\": {\n    \"tab_width\": 4,\n    \"lsp_servers\": {\n      \
         \"rust\": \"rust-analyzer\"\n    }\n  }\n}\n"
    );
    assert_eq!(project::read(&d).unwrap(), p, "and it reads back as itself");
}
