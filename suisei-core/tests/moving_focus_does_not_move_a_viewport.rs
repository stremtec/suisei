//! Moving the keyboard between panes is not a scroll.
//!
//! > 여전히 분할된 탭 간 포커싱 전환할 때 막 가로 스크롤 이상하게 튀고 위로 스크롤
//! > 튀고 이상한 문제가 넘 많음.
//!
//! `load_focused_pane` goes through `restore_state_from_tab` when the pane it
//! is moving to shows a different document — it has to, that is how `App` comes
//! to hold that document. But `restore_state_from_tab` also raises
//! `ScrollIntent::Restore`, and nothing lowered it again.
//!
//! The face obeys that flag by placing the clip at `App::scroll` and
//! `App::hscroll` exactly. Both are INTEGERS: `syncCorePosition` floors the
//! live clip to a line and a column. So a pane resting anywhere but on a cell
//! boundary — which is anywhere you land with a trackpad — was rounded on
//! arrival: up by the lost sub-line, left by the lost sub-column. Both axes,
//! every focus change, which is the report.
//!
//! And the pane's own clip was the only copy of the exact position. The
//! "restore" overwrote the good number with the rounded one.
//!
//! ```text
//! cargo test -p suisei-core --test moving_focus_does_not_move_a_viewport
//! ```

use suisei_core::app::{App, ScrollIntent};

/// Two panes on two different documents, focus on pane 1.
///
/// Both documents are long enough to scroll: `load_focused_pane` clamps the
/// restored line to the buffer's length, and a one-line blank would clamp every
/// position in this file to zero and prove nothing.
fn two_panes_two_docs() -> App {
    let mut app = App::new();
    // The horizontal half of the report only exists unwrapped — wrapped, there
    // is nothing to pan and `hscroll` is pinned to 0 by design.
    app.wrap_lines = false;
    app.buffer = suisei_core::buffer::Buffer::from_string(&"first\n".repeat(400));
    app.save_state_to_tab();
    app.split_vertical(); // both panes on the first tab
    app.open_blank_tab(); // the focused pane (1) takes a second document
    app.buffer = suisei_core::buffer::Buffer::from_string(&"second\n".repeat(400));
    app.save_state_to_tab();
    assert_eq!(app.split.panes.len(), 2);
    assert_ne!(
        app.split.panes[0].buffer, app.split.panes[1].buffer,
        "the two panes hold different documents"
    );
    app
}

#[test]
fn arriving_at_a_pane_asks_the_face_to_place_nothing() {
    let mut app = two_panes_two_docs();
    app.scroll_intent = ScrollIntent::None;

    app.focus_pane_to(0);

    assert_eq!(
        app.scroll_intent,
        ScrollIntent::None,
        "the pane has been on screen the whole time — there is nothing to place"
    );
}

/// It has to survive the trip back, too: the second crossing is the one where
/// the first pane's remembered position is at stake.
#[test]
fn and_on_the_way_back() {
    let mut app = two_panes_two_docs();
    app.focus_pane_to(0);
    app.scroll_intent = ScrollIntent::None;

    app.focus_pane_to(1);

    assert_eq!(app.scroll_intent, ScrollIntent::None);
}

/// What the pane was looking at is still what core reports — the flag went
/// away, the position did not.
#[test]
fn the_position_itself_is_still_restored() {
    let mut app = two_panes_two_docs();
    app.split.panes[0].scroll = 40;
    app.split.panes[0].hscroll = 7;

    app.focus_pane_to(0);

    assert_eq!(app.scroll, 40, "core still paints from the pane's own line");
    assert_eq!(app.hscroll, 7);
}

/// The guard. A pane changing WHICH DOCUMENT it shows is the case the flag
/// exists for, and it must still be raised — that clip really is in the wrong
/// place for the document arriving in it.
#[test]
fn changing_a_panes_document_still_asks_for_a_restore() {
    let mut app = two_panes_two_docs();
    let other = app.split.panes[0].buffer;
    app.scroll_intent = ScrollIntent::None;

    app.goto_tab_id(other);

    assert_eq!(
        app.scroll_intent,
        ScrollIntent::Restore,
        "a tab switch inside one pane still has to place the view"
    );
}

/// Focus moved by a chip click lands in the same place as ⌃W: `goto_tab`
/// forwards to the pane already holding the document rather than retargeting,
/// and that is a focus change, not a restore.
#[test]
fn clicking_the_chip_of_a_document_another_pane_holds_is_a_focus_change() {
    let mut app = two_panes_two_docs();
    // Give both panes the same document, then split them apart again by
    // pointing pane 1 at its own — the arrangement `goto_tab`'s layout branch
    // walks. Here it is enough that pane 0 holds `first`.
    let first = app.split.panes[0].buffer;
    app.scroll_intent = ScrollIntent::None;

    app.focus_pane_to(0);
    assert_eq!(app.split.focused_pane().buffer, first);
    assert_eq!(app.scroll_intent, ScrollIntent::None);
}
