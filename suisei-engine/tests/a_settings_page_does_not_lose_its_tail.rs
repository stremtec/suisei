//! Every settings row has to fit through the ABI, or a page goes missing.
//!
//! The snapshot writer clamps with `rows.len().min(SUISEI_MAX_SETTINGS_ROWS)`,
//! which is the only sane thing a writer with a fixed array can do — and it is
//! silent. The cap was 48 while the table was 65, so the last seventeen rows
//! never reached the face: fourteen languages fell off the end of Language
//! Servers, and Source Control lost all three of its rows and looked like a
//! page that existed for no reason.
//!
//! Nothing failed. No log, no error, no gap — the pages simply ended early,
//! and both lists grow whenever a theme or a language is added, so this would
//! have happened again at the next one. The arithmetic is the test.

use suisei_engine::ffi::SUISEI_MAX_SETTINGS_ROWS;

#[test]
fn settings_rows_fit_the_abi() {
    let panel = suisei_core::settings::SettingsPanel::new();
    let rows = panel.setting_rows();
    assert!(
        rows.len() <= SUISEI_MAX_SETTINGS_ROWS,
        "{} settings rows but the ABI carries {}. The last {} would be dropped \
         silently — raise SUISEI_MAX_SETTINGS_ROWS in ffi.rs AND in \
         suisei_engine.h, which is written by hand.",
        rows.len(),
        SUISEI_MAX_SETTINGS_ROWS,
        rows.len() - SUISEI_MAX_SETTINGS_ROWS
    );
}

/// The rows that were being dropped, named. A page whose every row is past the
/// cap is invisible — not degraded, invisible — so the two lists that end the
/// table are worth asserting on directly.
#[test]
fn source_control_reaches_the_face() {
    use suisei_core::settings::SettingRow;
    let panel = suisei_core::settings::SettingsPanel::new();
    let rows = panel.setting_rows();
    let carried = &rows[..rows.len().min(SUISEI_MAX_SETTINGS_ROWS)];
    for wanted in [SettingRow::OpenWorkbench, SettingRow::OpenScm] {
        assert!(
            carried.contains(&wanted),
            "{wanted:?} is past the ABI cap, so Settings → Source Control draws \
             nothing at all"
        );
    }
}

/// …and the language list, which is the other thing that grows.
#[test]
fn every_configurable_language_reaches_the_face() {
    let panel = suisei_core::settings::SettingsPanel::new();
    let rows = panel.setting_rows();
    let carried = rows.len().min(SUISEI_MAX_SETTINGS_ROWS);
    let langs = suisei_core::config::lsp_lang_catalog().len();
    let shown = rows[..carried]
        .iter()
        .filter(|r| matches!(r, suisei_core::settings::SettingRow::LspLang(_)))
        .count();
    assert_eq!(
        shown, langs,
        "Settings shows {shown} of {langs} languages; the rest are past the cap \
         and cannot be configured at all"
    );
}
