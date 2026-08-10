//! Reported bug: in the **grouped** state the editor split sometimes comes
//! undone, and grouped will not step back down to the loose state.
//!
//! Both were one deletion in the face. The strip's click dispatch used to read:
//!
//! ```text
//! if isLayoutDeskActive || editorSplit.isSplit { gotoTabId(doc) }
//! else                                         { activateLayout(group, doc) }
//! ```
//!
//! and the behaviour spec's "focus the member, arrangement unchanged" was
//! implemented as the first branch unconditionally — the condition looked like
//! a guess. It is not. `App::active_layout` is what `unfold_layout` operates
//! on, and only `activate_layout` sets it, so a click that skips activation
//! leaves the desk free: the parked split is never installed, and the
//! grouped -> loose step silently returns false.
//!
//! These assertions are on the ENGINE, so they hold whatever the face does with
//! them; what they pin is the state the face must put the engine in.
//!
//! ```text
//! cargo test -p suisei-engine --test grouped_member_click_installs_layout
//! ```

use suisei_engine::Engine;

fn seed_buffers(engine: &mut Engine, n: usize) -> Vec<u64> {
    let mut ids = Vec::new();
    for i in 0..n {
        if i > 0 {
            engine.open_blank_tab();
        }
        let idx = engine.app.tabs.buffers.len() - 1;
        engine.app.tabs.buffers[idx].filename = Some(std::path::PathBuf::from(format!(
            "/tmp/suisei_grouped_{i}.rs"
        )));
        ids.push(engine.app.tabs.buffers[idx].id.0);
    }
    ids
}

/// Fold two documents into a layout and return `(layout_id, member_a, member_b,
/// general)`, with the desk left FREE — a parked layout, which is the state the
/// bug needs.
fn parked_layout(engine: &mut Engine) -> (u64, u64, u64, u64) {
    let ids = seed_buffers(engine, 3);
    let (a, b, general) = (ids[0], ids[1], ids[2]);
    engine.goto_tab_id(a);
    engine.split_vertical();
    engine.goto_tab_id(b);
    assert!(engine.fold_layout(), "two distinct documents must fold");
    let layout_id = engine
        .app
        .layouts
        .first()
        .map(|l| l.id)
        .expect("a layout exists");
    // Leave the layout: the desk is now free and the layout is parked.
    engine.goto_tab_id(general);
    (layout_id, a, b, general)
}

#[test]
fn grouped_to_loose_needs_an_active_layout() {
    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    let (layout_id, _a, member_b, _general) = parked_layout(&mut engine);

    // A parked layout owns no desk, and `unfold_layout` is defined against the
    // ACTIVE layout — so the grouped -> loose step has nothing to act on. This
    // is the state the buggy click left the engine in.
    assert_eq!(engine.active_layout_id(), 0, "the layout is parked");
    assert!(
        !engine.unfold_layout(),
        "with no active layout there is nothing to unfold — this is the symptom"
    );

    // Activating is what makes the step available. `focus_doc` is the member
    // whose chip was clicked, so the arrangement returns with it in front.
    assert!(engine.activate_layout(layout_id, member_b));
    assert_eq!(
        engine.active_layout_id(),
        layout_id,
        "the desk is owned now"
    );
    assert!(
        engine.unfold_layout(),
        "GROUPED -> LOOSE must work once the layout owns the desk"
    );
    assert_eq!(engine.active_layout_id(), 0, "unfolding frees the desk");
}

#[test]
fn activating_a_parked_layout_installs_its_split() {
    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    let (layout_id, _a, member_b, _general) = parked_layout(&mut engine);

    // Parked: the desk shows the general document in ONE pane. Clicking a
    // member without activating leaves it that way, which is what "the split
    // came undone" was.
    assert_eq!(
        engine.app.split.pane_count(),
        1,
        "the parked desk is a single pane"
    );
    engine.goto_tab_id(member_b);
    assert_eq!(
        engine.app.split.pane_count(),
        1,
        "goto alone does NOT restore the arrangement — the deleted branch is why"
    );

    // Activating installs the parked tree, split and all.
    assert!(engine.activate_layout(layout_id, member_b));
    assert!(
        engine.app.split.pane_count() >= 2,
        "activating a folded layout must bring its split back, got {} pane(s)",
        engine.app.split.pane_count()
    );
}

#[test]
fn an_already_active_layout_focuses_in_place() {
    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    let (layout_id, member_a, member_b, _general) = parked_layout(&mut engine);
    assert!(engine.activate_layout(layout_id, member_b));
    let panes_before = engine.app.split.pane_count();

    // The other half of the rule: once the layout owns the desk, clicking a
    // sibling must move focus WITHOUT reinstalling the tree. Re-activating on
    // every click would rebuild the arrangement under the user mid-edit.
    engine.goto_tab_id(member_a);
    assert_eq!(
        engine.app.split.pane_count(),
        panes_before,
        "focusing a sibling must not disturb the arrangement"
    );
    assert_eq!(
        engine.active_layout_id(),
        layout_id,
        "and must not hand the desk back"
    );
}

/// Two layouts, so "is a layout active" and "is THIS layout active" differ.
///
/// Returns `(layout_a, layout_bc, member_b, member_c)` with A owning the desk —
/// the state the reported sequence starts from.
fn two_layouts(engine: &mut Engine) -> (u64, u64, u64, u64) {
    let ids = seed_buffers(engine, 4);
    let (a1, a2, b, c) = (ids[0], ids[1], ids[2], ids[3]);

    // Layout A, from a1 + a2.
    engine.goto_tab_id(a1);
    engine.split_vertical();
    engine.goto_tab_id(a2);
    assert!(engine.fold_layout(), "layout A folds");
    let layout_a = engine.app.layouts[0].id;

    // Leave it, then build layout BC from b + c.
    engine.goto_tab_id(b);
    engine.split_vertical();
    engine.goto_tab_id(c);
    assert!(engine.fold_layout(), "layout BC folds");
    let layout_bc = engine
        .app
        .layouts
        .iter()
        .map(|l| l.id)
        .find(|id| *id != layout_a)
        .expect("a second layout exists");

    // Put A back on the desk — the state the report starts from.
    assert!(engine.activate_layout(layout_a, a1));
    assert_eq!(engine.active_layout_id(), layout_a, "A owns the desk");
    (layout_a, layout_bc, b, c)
}

#[test]
fn clicking_a_member_of_another_layout_installs_that_layout() {
    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    let (layout_a, layout_bc, member_b, _c) = two_layouts(&mut engine);

    // The face's rule, as a question about the ENGINE: with A on screen and a
    // member of BC clicked, which call is correct? `goto_tab_id` is what the
    // old condition chose, because "some layout is active" was true.
    engine.goto_tab_id(member_b);
    assert_eq!(
        engine.app.split.pane_count(),
        1,
        "goto alone leaves BC uninstalled — core sees a target outside the \
         ACTIVE layout, parks A, and collapses the desk. This is the reported \
         'only B shows'."
    );
    assert_eq!(
        engine.active_layout_id(),
        0,
        "and no layout owns the desk now"
    );

    // Which is why the face must ask WHICH layout is active, not whether one
    // is, and activate when the answer is not this member's layout.
    assert!(engine.activate_layout(layout_bc, member_b));
    assert!(
        engine.app.split.pane_count() >= 2,
        "activating BC must bring its split, got {} pane(s)",
        engine.app.split.pane_count()
    );
    assert_ne!(
        engine.active_layout_id(),
        layout_a,
        "A is no longer on the desk"
    );
}
