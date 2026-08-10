//! Reported: switching from an ordinary tab to a layout tab makes the editor
//! shake for a moment.
//!
//! Two scroll positions get applied in one action. `App::activate_layout`
//! restores the parked tree and its panes, and says so:
//!
//! > Panes carry their own document and viewport, so restoring the tree and its
//! > panes restores the whole arrangement — including where each pane was
//! > scrolled to.
//!
//! Then `Engine::activate_layout` immediately calls `app.update_scroll()`,
//! which sets `scroll_intent = Caret`, drops `scroll_frac` to zero, and
//! recomputes the offset from the caret row. So the pane appears at its saved
//! viewport and is then moved to a caret-derived one — visible as a jump.
//!
//! The tab-switch path already knows this. `Engine::goto_tab_id` carries:
//!
//! > NO `update_scroll()` — same as `goto_tab`: the tab's saved scroll is
//! > authoritative; the caret-derived one snaps long scrolls to top.
//!
//! Layout activation is the same kind of restore and wants the same rule.
//!
//! ```text
//! cargo test -p suisei-engine --test activating_a_layout_keeps_its_scroll
//! ```

use suisei_engine::Engine;

/// A document long enough that a saved scroll and a caret-derived one differ.
fn long_text(tag: &str) -> String {
    (0..400)
        .map(|i| format!("{tag} line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn seed(engine: &mut Engine, n: usize) -> Vec<u64> {
    let mut ids = Vec::new();
    for i in 0..n {
        if i > 0 {
            engine.open_blank_tab();
        }
        let idx = engine.app.tabs.buffers.len() - 1;
        engine.app.tabs.buffers[idx].filename = Some(std::path::PathBuf::from(format!(
            "/tmp/suisei_scroll_{i}.rs"
        )));
        ids.push(engine.app.tabs.buffers[idx].id.0);
    }
    for (i, id) in ids.iter().enumerate() {
        engine.goto_tab_id(*id);
        engine.app.buffer = suisei_core::buffer::Buffer::from_string(&long_text(&format!("d{i}")));
    }
    ids
}

#[test]
fn activating_a_layout_restores_the_pane_viewport_it_parked() {
    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    let ids = seed(&mut engine, 3);
    let (a, b, general) = (ids[0], ids[1], ids[2]);

    // Build a split, scroll it somewhere that is NOT the top, and fold it.
    engine.goto_tab_id(a);
    engine.split_vertical();
    engine.goto_tab_id(b);
    engine.app.scroll = 180;
    engine.app.save_state_to_tab();
    assert!(engine.fold_layout(), "two distinct documents must fold");
    let layout_id = engine.app.layouts[0].id;

    // Park it by going to an ordinary tab, then come back to the layout.
    engine.goto_tab_id(general);
    assert!(engine.activate_layout(layout_id, b));

    // The arrangement came back; so must the place it was looking at. A
    // caret-derived recompute lands near row 0 here, because the caret never
    // moved off the first line — which is exactly the jump that reads as a
    // shake.
    assert_eq!(
        engine.app.scroll, 180,
        "activating a layout must restore the viewport it parked, not recompute \
         one from the caret — the pane is drawn at the saved offset and then \
         moved, which is the reported shake"
    );
}

#[test]
fn activating_a_layout_does_not_arm_a_caret_scroll() {
    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    let ids = seed(&mut engine, 3);
    let (a, b, general) = (ids[0], ids[1], ids[2]);

    engine.goto_tab_id(a);
    engine.split_vertical();
    engine.goto_tab_id(b);
    engine.app.scroll = 180;
    engine.app.save_state_to_tab();
    assert!(engine.fold_layout());
    let layout_id = engine.app.layouts[0].id;
    engine.goto_tab_id(general);

    // Clear whatever the tab switch left armed, so this observes activation
    // alone.
    engine.app.scroll_intent = suisei_core::app::ScrollIntent::None;
    assert!(engine.activate_layout(layout_id, b));

    // `Restore` is the right answer — the pane restore wants the face to apply
    // the saved offset. `Caret` is the wrong one: it means something decided to
    // chase the cursor instead, which is what moves the viewport a second time
    // after the panes are already on screen.
    assert_ne!(
        engine.app.scroll_intent,
        suisei_core::app::ScrollIntent::Caret,
        "a restore must not arm a caret-follow scroll"
    );
}
