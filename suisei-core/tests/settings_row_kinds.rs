//! Settings rows carry their identity, and those numbers are ABI.
//!
//! The GUI used to recover what a row MEANT by matching its display label:
//! `label == "LSP enabled"`, `label.hasPrefix("LSP ·")`, `label.contains("●")`,
//! and a hard-coded list of five toggle names. Core has always had the type
//! (`SettingRow`); it was flattened to strings at the FFI boundary and guessed
//! back in Swift. Two consequences, both real:
//!
//! * renaming a label in Core silently broke a control in the GUI, with nothing
//!   failing to say so;
//! * the three GPU rows were never in that list of five, so they existed in
//!   Core, toggled correctly, and could not be reached from the GUI at all.
//!
//! `kind()` is what the face now branches on, so these numbers are part of the
//! interface. A renumber must fail here rather than mis-address a control.
//!
//! ```text
//! cargo test -p suisei-core --test settings_row_kinds
//! ```

use std::collections::HashMap;
use suisei_core::settings::{
    SettingControl, SettingRow, SettingSurfacePage, SettingsPage, SettingsPanel,
};

/// Every row the Settings page lists, via the public panel.
fn rows() -> Vec<SettingRow> {
    SettingsPanel::new().setting_rows()
}

#[test]
fn kinds_are_stable_abi() {
    // Written out rather than derived: the point is to notice a change, and a
    // table generated from the enum would silently follow a renumber.
    let expected: &[(u32, &str)] = &[
        (1, "ThemeHeader"),
        (2, "Theme"),
        (3, "EditorHeader"),
        (4, "TabWidth"),
        (5, "RelativeNumber"),
        (6, "WrapLines"),
        (7, "UndoCaching"),
        (8, "ClipboardSync"),
        (9, "GpuAcc"),
        (10, "GpuGraphics"),
        (11, "GpuHyperlinks"),
        (12, "KeyHints"),
        (13, "LspHeader"),
        (14, "LspEnabled"),
        (15, "LspLang"),
        (16, "GitHeader"),
        (17, "OpenWorkbench"),
        (18, "OpenScm"),
        (19, "HighlightColor"),
        (20, "UpdateCheck"),
        (21, "AppearanceMode"),
        (22, "GlassStyle"),
    ];
    let actual: HashMap<u32, ()> = rows().iter().map(|r| (r.kind(), ())).collect();
    for (code, name) in expected {
        assert!(
            actual.contains_key(code),
            "kind {code} ({name}) is no longer emitted — the face addresses controls by \
             this number, so removing or renumbering it mis-targets a control"
        );
    }
}

#[test]
fn no_setting_row_is_untyped() {
    // 0 means "prose, no setting behind it". A real setting reporting 0 would
    // be invisible to the face's typed lookups — exactly the failure this
    // replaced, just in a new spelling.
    for row in rows() {
        assert_ne!(
            row.kind(),
            0,
            "{row:?} reports kind 0, which the face reads as a prose row and skips"
        );
    }
}

#[test]
fn indexed_rows_carry_their_index() {
    let themes: Vec<u32> = rows()
        .iter()
        .filter(|r| matches!(r, SettingRow::Theme(_)))
        .map(|r| r.payload())
        .collect();
    assert!(!themes.is_empty(), "there is at least one theme");
    assert_eq!(
        themes,
        (0..themes.len() as u32).collect::<Vec<_>>(),
        "theme rows must carry their own index, in order"
    );

    let langs: Vec<u32> = rows()
        .iter()
        .filter(|r| matches!(r, SettingRow::LspLang(_)))
        .map(|r| r.payload())
        .collect();
    assert_eq!(
        langs,
        (0..langs.len() as u32).collect::<Vec<_>>(),
        "language rows must carry their catalog index, in order"
    );
}

#[test]
fn the_gpu_rows_are_listed() {
    // The concrete regression: these three are toggles with working handlers in
    // Core that the GUI never showed, because the GUI's list of toggles was
    // five label strings typed by hand.
    let kinds: Vec<u32> = rows().iter().map(|r| r.kind()).collect();
    for (code, name) in [
        (9, "GPU accel"),
        (10, "GPU graphics"),
        (11, "GPU hyperlinks"),
    ] {
        assert!(
            kinds.contains(&code),
            "{name} (kind {code}) must be listed — it was unreachable from the GUI"
        );
    }
}

#[test]
fn native_general_is_not_a_wall_of_switches() {
    let general: Vec<_> = rows()
        .into_iter()
        .filter(|row| row.presentation().page == SettingSurfacePage::General)
        .collect();

    assert!(
        !general.is_empty(),
        "General has editor defaults to present"
    );
    assert_eq!(
        general
            .iter()
            .filter(|row| row.presentation().control == SettingControl::Toggle)
            .count(),
        0,
        "mode-like booleans in General must be menus/segments, not switches"
    );
    assert!(
        general
            .iter()
            .any(|row| { row.presentation().control == SettingControl::Menu })
    );
    assert!(
        general
            .iter()
            .all(|row| { row.presentation().control != SettingControl::Segmented }),
        "tiny number-only segmented controls make preferences harder to scan"
    );
    assert_eq!(
        SettingRow::TabWidth.presentation().control,
        SettingControl::Menu,
        "tab width is a discrete choice, presented as a checked native menu"
    );
}

#[test]
fn appearance_has_one_constrained_customisation() {
    let row = SettingRow::HighlightColor.presentation();
    assert_eq!(row.page, SettingSurfacePage::Appearance);
    assert_eq!(row.control, SettingControl::Color);

    let mut panel = SettingsPanel::new();
    panel.set_highlight_color("ff2d55");
    assert_eq!(panel.draft.highlight_color, "#FF2D55");
    panel.set_highlight_color("default");
    assert_eq!(panel.draft.highlight_color, "default");
}

#[test]
fn appearance_modes_are_explicit_and_persistable() {
    assert_eq!(
        SettingRow::AppearanceMode.presentation().options,
        "Automatic|Light|Dark"
    );
    assert_eq!(
        SettingRow::GlassStyle.presentation().options,
        "Clear|Tinted"
    );

    let rows = rows();
    let mut panel = SettingsPanel::new();
    panel.open_panel();
    panel.page = SettingsPage::Setting;

    panel.selected = rows
        .iter()
        .position(|row| *row == SettingRow::AppearanceMode)
        .expect("appearance-mode row");
    panel.set_value(2);
    assert_eq!(panel.draft.theme, "dark");

    panel.selected = rows
        .iter()
        .position(|row| *row == SettingRow::GlassStyle)
        .expect("glass-style row");
    panel.set_value(1);
    assert_eq!(panel.draft.glass_style, "tinted");
}

#[test]
fn terminal_only_controls_are_advanced() {
    for row in [
        SettingRow::GpuAcc,
        SettingRow::GpuGraphics,
        SettingRow::GpuHyperlinks,
    ] {
        let presentation = row.presentation();
        assert_eq!(presentation.page, SettingSurfacePage::Editor);
        assert!(
            presentation.advanced,
            "{row:?} should be collapsed by default"
        );
    }
}

#[test]
fn native_picker_sets_an_explicit_value() {
    let mut panel = SettingsPanel::new();
    panel.open_panel();
    panel.page = SettingsPage::Setting;
    panel.selected = rows()
        .iter()
        .position(|row| *row == SettingRow::TabWidth)
        .expect("tab-width row");

    panel.draft.tab_width = 2;
    panel.set_value(2);
    assert_eq!(
        panel.draft.tab_width, 8,
        "2 → option 8 is direct, not a cycle to 4"
    );
}
