//! Wrapped, the row you can count on screen is not the line it belongs to.
//!
//! > 여전히 분할된 탭 간 포커싱 전환할 때 막 가로 스크롤 이상하게 튀고 위로 스크롤
//! > 튀고 이상한 문제가 넘 많음.
//! > 스크롤 이슈는 no wrapping 옵션 키면 귀신같이 사라짐.
//!
//! That last sentence is the whole diagnosis. The face derived its scroll
//! position as `documentVisibleRect.minY / lineHeight` — the number of DRAWN
//! rows above the clip — and handed it to core as `App::scroll`, which is a
//! LINE index. Unwrapped the two are the same number, so nothing showed for as
//! long as nothing wrapped. Wrapped, every sync pushed core's idea of the pane
//! further past the truth, `park_focused_pane` stored that, and the restore on
//! the way back converted it the other way through the wrap map and placed the
//! view somewhere the user had never been.
//!
//! Painting never showed it: the canvas draws from its own clip through
//! `nearestBufferRow(atY:)`, which converts. Core's copy is only read back when
//! a pane is restored — which is exactly when the jump was reported.
//!
//! The conversion existed the whole time. `wrap_buffer_at` is even documented
//! as "for turning a click or a viewport top back into a line", and the
//! viewport half had no caller. This pins the round trip it is now used for.
//!
//! ```text
//! cargo test -p suisei-core --test a_visual_row_is_not_a_line
//! ```

use suisei_core::wrap::WrapMap;

/// Lines of wildly different lengths, so visual rows and line numbers come
/// apart immediately and stay apart.
fn ragged() -> Vec<String> {
    (0..200)
        .map(|i| match i % 4 {
            0 => String::new(),
            1 => "short".to_string(),
            2 => "x".repeat(140),
            _ => "y".repeat(37),
        })
        .collect()
}

fn map_of(lines: &[String], cols: u16) -> WrapMap {
    WrapMap::build(lines, 1, cols, 4, 2, None)
}

/// The two directions are inverses at every line: the first visual row of a
/// line belongs to that line. This is the property the face's scroll sync now
/// leans on.
#[test]
fn the_first_visual_row_of_a_line_belongs_to_that_line() {
    let lines = ragged();
    let map = map_of(&lines, 40);
    for row in 0..lines.len() {
        let v = map.visual_of(row);
        let (back, seg) = map.buffer_at(v);
        assert_eq!(back, row, "line {row} came back as {back} via visual row {v}");
        assert_eq!(seg, 0, "and as its own first segment");
    }
}

/// Every visual row in the document names a line, and the rows of one line are
/// consecutive — nothing between them belongs to a neighbour.
#[test]
fn every_drawn_row_names_the_line_it_draws() {
    let lines = ragged();
    let map = map_of(&lines, 40);
    let (mut row, mut seg) = (0usize, 0u32);
    assert_eq!(map.buffer_at(0), (0, 0), "the top row is the first line");
    for v in 1..map.total_rows() {
        let (r, s) = map.buffer_at(v);
        if r == row {
            // Still inside the same line: the next segment of it, in order.
            assert_eq!(s, seg + 1, "at visual row {v}, line {r}");
        } else {
            assert_eq!(r, row + 1, "lines are walked in order, at visual row {v}");
            assert_eq!(s, 0, "a line starts at its own first segment");
        }
        (row, seg) = (r, s);
    }
    assert_eq!(row, lines.len() - 1, "walked the whole document");
}

/// The mistake itself, stated as a number: reading a visual row as a line
/// misses by more than a screenful long before the end of an ordinary file.
///
/// This is what makes it a jump rather than a wobble — and why the same code
/// is exactly right with wrapping off.
#[test]
fn reading_a_visual_row_as_a_line_is_off_by_a_screenful() {
    let lines = ragged();
    let map = map_of(&lines, 40);

    // Halfway down the drawn document.
    let v = map.total_rows() / 2;
    let (real_line, _) = map.buffer_at(v);
    let drift = v as usize - real_line;
    assert!(
        drift > 30,
        "visual row {v} is line {real_line} — a drift of {drift} rows"
    );

    // Unwrapped, the same walk is the identity, which is the whole reason this
    // survived so long.
    let flat = map_of(&lines, 0);
    for row in 0..lines.len() {
        assert_eq!(flat.visual_of(row) as usize, row);
        assert_eq!(flat.buffer_at(row as u32), (row, 0));
    }
}
