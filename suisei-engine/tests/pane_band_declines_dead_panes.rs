//! Reported: leaving a split for an ordinary tab flashes the destination in
//! BOTH panes before the split collapses — "A | B" becomes "C | C" and only
//! then one pane. Fast, but visible.
//!
//! The engine collapses in a single call, so the flash is not a half-applied
//! state. It is `build_editor_band` answering for a pane that no longer exists:
//! once `collapse_to` has run, `is_split()` is false and the old arm returned
//! the CURRENT document for every index, while the split arm clamped with
//! `pane.min(n - 1)`. The face animates the pane list over 0.22s, so the
//! departing pane view is still alive and still pulling — and it was handed the
//! destination document.
//!
//! ```text
//! cargo test -p suisei-engine --test pane_band_declines_dead_panes
//! ```

use suisei_engine::Engine;

fn seed_buffers(engine: &mut Engine, n: usize) -> Vec<u64> {
    let mut ids = Vec::new();
    for i in 0..n {
        if i > 0 {
            engine.open_blank_tab();
        }
        let idx = engine.app.tabs.buffers.len() - 1;
        engine.app.tabs.buffers[idx].filename =
            Some(std::path::PathBuf::from(format!("/tmp/suisei_band_{i}.rs")));
        ids.push(engine.app.tabs.buffers[idx].id.0);
    }
    // Distinct text per document, so "which one is this pane showing?" is
    // decidable from the band alone. Written through the LIVE buffer: `App`'s
    // live fields ARE the focused document (tabs.rs S2), and `tabs.buffers[i]`
    // is only the parked copy — assigning there is undone by the next
    // `save_state_to_tab`.
    for (i, id) in ids.iter().enumerate() {
        engine.goto_tab_id(*id);
        engine.app.buffer = suisei_core::buffer::Buffer::from_string(&format!("doc{i}"));
    }
    ids
}

fn first_row_text(engine: &Engine, pane: usize) -> Option<String> {
    let (lines, _) = engine.editor_band(pane, 0, 4, 0, 200);
    lines.first().map(|l| l.text.clone())
}

#[test]
fn a_pane_that_does_not_exist_gets_no_rows() {
    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    seed_buffers(&mut engine, 3);

    // Unsplit: pane 0 is the desk, and nothing else is.
    assert_eq!(engine.app.split.pane_count(), 1);
    assert!(
        first_row_text(&engine, 0).is_some(),
        "the one live pane must still answer"
    );
    assert!(
        first_row_text(&engine, 1).is_none(),
        "pane 1 does not exist — it must not be handed pane 0's document"
    );
    assert!(first_row_text(&engine, 7).is_none(), "nor should pane 7");
}

#[test]
fn collapsing_a_split_does_not_echo_the_destination_into_the_dead_pane() {
    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    let ids = seed_buffers(&mut engine, 3);
    let (a, b, c) = (ids[0], ids[1], ids[2]);

    // A | B, folded into a layout so the desk owns it.
    engine.goto_tab_id(a);
    engine.split_vertical();
    engine.goto_tab_id(b);
    assert!(engine.fold_layout(), "two distinct documents must fold");
    assert_eq!(engine.app.split.pane_count(), 2, "the desk is split");

    let pane0 = first_row_text(&engine, 0);
    let pane1 = first_row_text(&engine, 1);
    assert!(
        pane0.is_some() && pane1.is_some(),
        "both panes answer while split"
    );
    assert_ne!(pane0, pane1, "and they show DIFFERENT documents");

    // Leave the layout for an ordinary tab: the split collapses in one call.
    engine.goto_tab_id(c);
    assert_eq!(
        engine.app.split.pane_count(),
        1,
        "leaving collapses the desk"
    );

    // This is the assertion the bug fails. The face's departing pane view is
    // still alive for the length of its animation and still pulls pane 1; it
    // must be told there is nothing there, NOT handed C.
    let dead = first_row_text(&engine, 1);
    assert!(
        dead.is_none(),
        "pane 1 is gone; it must not render the destination document — got {dead:?}"
    );

    // And the surviving pane does show the destination, so declining above is
    // not just breaking everything equally.
    let live = first_row_text(&engine, 0).expect("pane 0 is live");
    assert!(
        live.starts_with("doc2"),
        "pane 0 must show the tab we switched to, got {live:?}"
    );
}
