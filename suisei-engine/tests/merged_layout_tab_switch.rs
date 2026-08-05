//! Reported bug: with a layout in the **merged** (unified) state, clicking any
//! other tab does nothing — you must revert the layout to *grouped* first.
//!
//! Three strip states exist:
//!   general — an ordinary document chip, no layout
//!   grouped — the layout's members keep their own chips inside a container
//!   merged  — the layout collapses to ONE chip carrying the layout's name
//!
//! Switching works in general and grouped, and fails in merged. This test
//! reproduces the sequence headlessly so the failure can be pinned to the
//! engine or ruled out of it, without driving the GUI.
//!
//! ```text
//! cargo test -p suisei-engine --test merged_layout_tab_switch
//! ```

use suisei_engine::Engine;

/// Open `n` distinct named buffers and return their ids in strip order.
fn seed_buffers(engine: &mut Engine, n: usize) -> Vec<u64> {
    let mut ids = Vec::new();
    for i in 0..n {
        if i > 0 {
            engine.open_blank_tab();
        }
        let idx = engine.app.tabs.buffers.len() - 1;
        // A layout needs distinct documents; give each buffer a real path.
        engine.app.tabs.buffers[idx].filename =
            Some(std::path::PathBuf::from(format!("/tmp/suisei_merge_{i}.rs")));
        ids.push(engine.app.tabs.buffers[idx].id.0);
    }
    ids
}

fn current_buffer_id(engine: &Engine) -> u64 {
    engine.app.tabs.buffers[engine.app.current_buffer()].id.0
}

/// The whole reported sequence: fold two documents into a layout, merge it,
/// then try to reach a tab that is not one of its members.
#[test]
fn a_merged_layout_does_not_trap_the_tab_strip() {
    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);

    // Three documents: two will form the layout, one stays general.
    let ids = seed_buffers(&mut engine, 3);
    let (member_a, member_b, general) = (ids[0], ids[1], ids[2]);

    // Put the two members on their own panes and fold them into a layout.
    engine.goto_tab_id(member_a);
    engine.split_vertical();
    engine.goto_tab_id(member_b);
    assert!(engine.fold_layout(), "two distinct documents must fold");

    let layout_id = engine
        .app
        .layouts
        .first()
        .map(|l| l.id)
        .expect("a layout exists after folding");

    // Grouped: switching to the general tab must work. This is the control —
    // if it fails, the test setup is wrong rather than the feature.
    engine.goto_tab_id(general);
    assert_eq!(
        current_buffer_id(&engine),
        general,
        "GROUPED: clicking a general tab must switch to it"
    );

    // Back into the layout, then MERGE it (grouped -> unified).
    engine.activate_layout(layout_id, 0);
    assert!(
        engine.toggle_layout_style(layout_id),
        "the layout must accept a style toggle"
    );
    assert_eq!(
        engine.app.layouts[0].style,
        suisei_core::layout_tab::LayoutStyle::Unified,
        "the layout is now MERGED"
    );

    // The reported failure: from here, no other tab can be reached.
    engine.goto_tab_id(general);
    assert_eq!(
        current_buffer_id(&engine),
        general,
        "MERGED: clicking a general tab must switch to it, exactly as it does \
         when the same layout is grouped"
    );
    assert_eq!(
        engine.app.active_layout, None,
        "leaving a merged layout must clear the active desk, or the next \
         switch is judged against a layout the user already left"
    );
}

/// The strip must still OFFER a route out. If the merged chip is the only
/// non-member entry the face can address, no click can ever leave.
#[test]
fn a_merged_layout_still_publishes_the_other_tabs() {
    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    let ids = seed_buffers(&mut engine, 3);
    let general = ids[2];

    engine.goto_tab_id(ids[0]);
    engine.split_vertical();
    engine.goto_tab_id(ids[1]);
    assert!(engine.fold_layout());
    let layout_id = engine.app.layouts[0].id;
    engine.activate_layout(layout_id, 0);
    assert!(engine.toggle_layout_style(layout_id));
    engine.tick(50);

    let chrome = engine.last_diff.chrome.as_ref().expect("composed");
    // Exactly one chip for the layout, and the general document still there.
    let layout_chips = chrome.tabs.iter().filter(|t| t.is_layout).count();
    assert_eq!(layout_chips, 1, "merged layout emits exactly one chip");
    assert!(
        chrome.tabs.iter().any(|t| !t.is_layout && t.id == general),
        "the general tab must remain addressable on the strip while a layout \
         is merged — its stable id is what a chip click sends back"
    );
}
