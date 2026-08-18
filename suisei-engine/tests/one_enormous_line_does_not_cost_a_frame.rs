//! A line the editor can only show 480 columns of must only cost 480 columns.
//!
//! Reported against an 8.5 MB HTML session dump: "1600~~ 라인부터 렌더링이
//! 안되고 끊김". The engine was not stopping. Line 1165 of that file is a
//! minified `<script>` — **8,211,745 characters on one line** — and every band
//! pull that touched it took 481 ms: 132 ms to expand its tabs into a fresh
//! copy of all 8 MB, and 346 ms for the span pass, which converts a byte
//! offset to a visual column by walking the line from the start, once per
//! span. Both to draw the first 480 characters, which is all a row ever shows.
//!
//! At a frame and a half per row, scrolling through that region is indeed a
//! renderer that has stopped.
//!
//! ```text
//! cargo test -p suisei-engine --test one_enormous_line_does_not_cost_a_frame
//! ```

use std::time::{Duration, Instant};
use suisei_engine::Engine;
use suisei_engine::compositor::build_editor_band;

/// Eight megabytes on one line, with ordinary lines around it — the shape of
/// the file that reported this, and `.html` for the same reason: the cost was
/// in the span pass, and the span pass only has spans when there is a grammar.
///
/// (A single line this long in a `.js` file is a different problem and not
/// this one: the JavaScript grammar itself does not finish. Worth knowing,
/// worth fixing elsewhere.)
fn engine_with_a_monster(name: &str) -> (Engine, usize) {
    // One directory per test: these run in parallel and each writes eight
    // megabytes, so a shared path is a torn read waiting to happen.
    let dir = std::env::temp_dir().join(format!("suisei_enormous_line/{name}"));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("huge.html");

    let mut text = String::new();
    for i in 0..40 {
        text.push_str(&format!("const line{i} = {i};\n"));
    }
    let monster_row = 40;
    text.push_str(&"a.b(1),".repeat(1_200_000));
    text.push('\n');
    for i in 0..40 {
        text.push_str(&format!("const after{i} = {i};\n"));
    }
    std::fs::write(&path, &text).expect("write");

    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    engine.app = suisei_core::app::App::open_file(path.to_str().unwrap());
    (engine, monster_row)
}

#[test]
fn a_band_holding_a_megabyte_long_line_still_lands_in_a_frame() {
    let (engine, monster) = engine_with_a_monster("band");
    assert!(
        engine.app.buffer.line(monster).len() > 8_000_000,
        "the line really is enormous"
    );

    // Warm: the first pull of any band pays for whatever it caches.
    let _ = build_editor_band(&engine.app, 0, 0, 40, 0, 200);

    let t = Instant::now();
    let (lines, _) = build_editor_band(&engine.app, 0, monster, 40, 0, 200);
    let took = t.elapsed();

    assert!(!lines.is_empty(), "the rows are there");
    assert!(
        took < Duration::from_millis(200),
        "a band holding one enormous line took {took:?} — something is \
         measuring the whole line to draw 480 columns of it"
    );
}

/// Wrapped, which is where it was found: with soft wrap on, the chunker runs
/// over the same text and the span pass runs once per chunk.
#[test]
fn the_same_line_is_cheap_when_it_is_wrapped() {
    let (mut engine, monster) = engine_with_a_monster("wrapped");
    engine.app.wrap_lines = true;
    let _ = build_editor_band(&engine.app, 0, 0, 40, 150, 200);

    let t = Instant::now();
    let (lines, _) = build_editor_band(&engine.app, 0, monster, 40, 150, 200);
    let took = t.elapsed();

    assert!(!lines.is_empty());
    assert!(
        took < Duration::from_millis(200),
        "wrapped, it took {took:?}"
    );
}

/// The cut itself is unchanged: what a row shows is the head of the line and
/// an ellipsis saying there is more. This is the contract the speed above
/// depends on — everything is allowed to stop measuring at the cut precisely
/// because nothing past it is ever drawn.
#[test]
fn a_cut_row_says_it_was_cut() {
    let (engine, monster) = engine_with_a_monster("cut");
    let (lines, _) = build_editor_band(&engine.app, 0, monster, 2, 0, 200);
    let row = &lines[0];
    assert!(row.text.starts_with("a.b(1),a.b(1),"), "the head is real text");
    assert!(row.text.ends_with('…'), "and it says it was cut: {:?}", &row.text);
    assert!(row.text.chars().count() <= 481);

    // And the ordinary lines around it are untouched.
    let (before, _) = build_editor_band(&engine.app, 0, 0, 3, 0, 200);
    assert_eq!(before[0].text, "const line0 = 0;");
    let (after, _) = build_editor_band(&engine.app, 0, monster + 1, 2, 0, 200);
    assert_eq!(after[0].text, "const after0 = 0;");
}

/// A selection that covers the enormous line is measured against the drawn
/// prefix too — `select all` used to walk eight million characters per pull to
/// place a band that ends at column 480 either way.
#[test]
fn selecting_everything_does_not_walk_the_monster() {
    let (mut engine, monster) = engine_with_a_monster("selected");
    engine.select_all();
    let _ = build_editor_band(&engine.app, 0, 0, 40, 0, 200);

    let t = Instant::now();
    let (lines, _) = build_editor_band(&engine.app, 0, monster, 8, 0, 200);
    let took = t.elapsed();

    assert!(
        lines[0].sel_v0.is_some(),
        "the row is selected, and says so"
    );
    assert!(took < Duration::from_millis(200), "took {took:?}");
}
