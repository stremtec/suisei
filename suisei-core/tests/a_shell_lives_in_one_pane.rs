//! One shell, one pane.
//!
//! Reported twice, as two bugs:
//!
//! > 터미널에서 뭐 cline, opencode, claude 같은거 실행하구 스플릿하면 로직 꼬여서
//! > 터미널이 스플릿되거나 문서가 스플릿되는게 아님.
//!
//! > 분할 상태에서 A페인에 터미널 열고 클로드, B페인에 터미널 열고 codex를 열었다
//! > 치자. 이따 A 페인도 codex 탭으로 바꿔버리면 그대로 B pane 은 검정색으로
//! > 가득차고 작동을 안함.
//!
//! One cause. `split_focused` copies the focused pane's slot into the new pane
//! — the VS Code rule, and right for a file, because two views of one document
//! is what a split is FOR. A terminal is not a document: there is one shell
//! behind it and one `PaneTerminalView` of that shell, and the face's
//! `TerminalHostView.mount` moves that view to whichever pane asked last. So
//! the second pane took the screen and the first went black, with its process
//! still running behind nothing.
//!
//! Everything downstream followed from that one state. With both panes reading
//! as terminals, ⌃⇧T saw "already showing a terminal" and closed the shell
//! instead of opening a second one; closing either pane looked like it had
//! killed a terminal that was never really in two places.
//!
//! ```text
//! cargo test -p suisei-core --test a_shell_lives_in_one_pane
//! ```

use suisei_core::app::App;

/// Which panes are showing a shell.
fn terminal_panes(app: &App) -> Vec<usize> {
    app.split
        .panes
        .iter()
        .enumerate()
        .filter(|(_, p)| app.is_terminal_tab(p.buffer))
        .map(|(i, _)| i)
        .collect()
}

fn terminal_tabs(app: &App) -> usize {
    app.tabs.buffers.iter().filter(|t| t.terminal.is_some()).count()
}

#[test]
fn splitting_a_terminal_pane_leaves_the_shell_where_it_was() {
    let mut app = App::new();
    app.toggle_terminal_full();
    let shell = app.split.focused_pane().buffer;

    app.split_vertical();

    assert_eq!(app.split.panes.len(), 2);
    assert_eq!(
        terminal_panes(&app),
        vec![0],
        "the shell stayed in the pane that had it, and only there"
    );
    assert_ne!(
        app.split.focused_pane().buffer,
        shell,
        "the new pane is not a second view of the same process"
    );
    assert_eq!(terminal_tabs(&app), 1, "and no shell was forked to fill it");
}

/// `⌘\` and "Split Above" go through different entry points; the rule is about
/// the split, not about which direction it went.
#[test]
fn every_split_direction_keeps_the_rule() {
    let directions: [(&str, fn(&mut App)); 4] = [
        ("vertical", App::split_vertical),
        ("horizontal", App::split_horizontal),
        ("above", App::split_above),
        ("left", App::split_left),
    ];
    for (name, split) in directions {
        let mut app = App::new();
        app.toggle_terminal_full();
        split(&mut app);
        assert_eq!(
            terminal_panes(&app).len(),
            1,
            "{name}: {} panes show the shell",
            terminal_panes(&app).len()
        );
    }
}

/// The pane shows the document the shell displaced — which is what was there a
/// moment ago, and what `terminal_replaced` has been holding since.
#[test]
fn the_new_pane_gets_the_document_the_terminal_displaced() {
    let mut app = App::new();
    let before = app.split.focused_pane().buffer;
    app.toggle_terminal_full();
    app.split_vertical();

    assert_eq!(
        app.split.focused_pane().buffer, before,
        "the new pane went back to what the terminal took the pane from"
    );
    // And `App` is really holding it, not just pointing at it. Retargeting a
    // pane without the park-then-load around it is how this file's other bugs
    // began — a slot naming one document while the live buffer holds another.
    assert_eq!(
        app.current_buffer_id(),
        before,
        "the document was loaded, not just named"
    );
}

/// The gesture that used to close the shell. With the panes untangled ⌃⇧T is
/// read for what it is again: this pane has no terminal, so open one.
#[test]
fn a_second_terminal_is_what_the_new_pane_asks_for() {
    let mut app = App::new();
    app.toggle_terminal_full();
    app.split_vertical();

    app.toggle_terminal_full();

    assert_eq!(terminal_tabs(&app), 2, "two shells, one per pane");
    assert_eq!(terminal_panes(&app), vec![0, 1]);
    assert_ne!(
        app.split.panes[0].buffer, app.split.panes[1].buffer,
        "and they are different shells"
    );
}

/// The second report. Clicking a terminal chip that another pane is showing
/// GOES there; it does not take the screen away from it.
#[test]
fn showing_a_terminal_another_pane_has_moves_focus_to_that_pane() {
    let mut app = App::new();
    app.toggle_terminal_full(); // pane 0: shell A
    app.split_vertical();
    app.toggle_terminal_full(); // pane 1: shell B
    let a = app.split.panes[0].buffer;
    let b = app.split.panes[1].buffer;
    assert_eq!(app.split.focus_index(), 1);

    app.goto_tab_id(a);

    assert_eq!(app.split.focus_index(), 0, "focus went to the pane holding A");
    assert_eq!(app.split.panes[0].buffer, a);
    assert_eq!(
        app.split.panes[1].buffer, b,
        "and B kept its screen — this is the black pane"
    );
}

/// The rule is about the process, not about tabs. A document may be on as many
/// panes as the user likes, and taking that away would break the split.
#[test]
fn two_panes_may_still_show_one_document() {
    let mut app = App::new();
    let doc = app.split.focused_pane().buffer;
    app.split_vertical();

    assert_eq!(app.split.panes[0].buffer, doc);
    assert_eq!(app.split.panes[1].buffer, doc, "files still duplicate");

    // And clicking its chip while another pane shows it still retargets the
    // focused pane rather than jumping away.
    app.goto_tab_id(doc);
    assert_eq!(app.split.focus_index(), 1);
}

/// Every tab being a terminal is a state the user can reach, and a pane still
/// has to show something.
#[test]
fn a_pane_split_off_the_last_terminal_still_gets_a_document() {
    let mut app = App::new();
    let blank = app.split.focused_pane().buffer;
    app.toggle_terminal_full();
    app.close_tab_id(blank);
    assert_eq!(
        app.tabs.buffers.iter().filter(|t| t.terminal.is_none()).count(),
        0,
        "only the shell is open"
    );

    app.split_vertical();

    assert_eq!(terminal_panes(&app).len(), 1, "still only one pane on the shell");
    assert!(
        !app.is_terminal_tab(app.split.focused_pane().buffer),
        "the new pane got a document"
    );
}
