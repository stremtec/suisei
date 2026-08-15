//! Per-token theme editing: what it changes, what it must not, and the reason
//! the palette layer used to refuse it.
//!
//! `theme::with_highlight` argued the refusal like this: "Syntax colours and
//! surfaces stay authored as a coherent Light or Dark palette. Allowing each of
//! those colours to drift independently recreates the low-contrast theme
//! combinations this layer exists to prevent."
//!
//! The reason was sound; the conclusion was too strong. You cannot let someone
//! choose their own colours and also guarantee those colours are readable — but
//! you can measure it and say so. `contrast_ratio` is that measurement, and
//! these tests pin both halves: the edit lands exactly where asked, and the
//! ratio still reports honestly when the edit is a bad one.
//!
//! ```text
//! cargo test -p suisei-core --test theme_overrides
//! ```

use std::collections::BTreeMap;
use suisei_core::config::Config;
use suisei_core::settings::{SettingsAction, SettingsPanel};
use suisei_core::theme::{self, ThemeToken};

fn overrides(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// Every token addresses exactly the field it names, and touches no other.
#[test]
fn a_token_sets_one_colour_and_only_that_colour() {
    let base = theme::resolve("ocean", true);
    for token in ThemeToken::ALL {
        let map = overrides(&[(token.key(), "#123456")]);
        let edited = theme::with_overrides(base, Some(&map));

        assert_eq!(
            (token.get(&edited).r, token.get(&edited).g, token.get(&edited).b),
            (0x12, 0x34, 0x56),
            "{} did not take the colour it was given",
            token.label()
        );

        for other in ThemeToken::ALL.iter().filter(|t| *t != token) {
            assert_eq!(
                other.get(&edited),
                other.get(base),
                "editing {} also moved {}",
                token.label(),
                other.label()
            );
        }
    }
}

/// Keys are ABI: a user's saved colour is addressed by name in their config,
/// and renaming one silently orphans it. Written out rather than derived.
#[test]
fn token_keys_are_stable() {
    let expected = [
        "fg",
        "comment",
        "string",
        "number",
        "keyword",
        "type_name",
        "function",
        "macro_name",
        "namespace",
        "parameter",
        "property",
        "constant",
        "operator",
        "punctuation",
        "line_no",
        "editor_bg",
        "selection_bg",
        "cursor",
        "status_bg",
        "accent",
        // Appended, not inserted. `current_line` reads as belonging beside
        // `line_no` and `invisibles` beside the other ink, and neither may go
        // there: the index is how the face addresses a token, so putting them
        // in their "natural" place would repoint every index after it. This
        // test is what caught exactly that.
        "current_line",
        "invisibles",
    ];
    let actual: Vec<&str> = ThemeToken::ALL.iter().map(|t| t.key()).collect();
    assert_eq!(
        actual, expected,
        "token keys AND their order are ABI — append, never insert"
    );

    for key in expected {
        assert!(ThemeToken::from_key(key).is_some(), "{key} must round-trip");
    }
    assert!(ThemeToken::from_key("no_such_token").is_none());
}

/// A colour well hands back an opaque value. Several surfaces composite over
/// what is behind them — that is why `Rgba` carries alpha at all — so taking
/// the well's alpha would turn a translucent selection into a solid slab.
#[test]
fn an_override_keeps_the_authored_alpha() {
    let base = theme::resolve("dark", true);
    let translucent: Vec<_> = ThemeToken::ALL
        .iter()
        .filter(|t| t.get(base).a != 255)
        .collect();

    for token in translucent {
        let map = overrides(&[(token.key(), "#FF0000")]);
        let edited = theme::with_overrides(base, Some(&map));
        assert_eq!(
            token.get(&edited).a,
            token.get(base).a,
            "{} lost its alpha to the colour well",
            token.label()
        );
    }
}

/// One bad line must not cost the user the other nineteen colours.
#[test]
fn a_damaged_override_is_skipped_not_fatal() {
    let base = theme::resolve("nord", true);
    let map = overrides(&[
        ("keyword", "#FF00FF"),
        ("comment", "not-a-colour"),
        ("no_such_token", "#00FF00"),
        ("string", ""),
    ]);
    let edited = theme::with_overrides(base, Some(&map));

    assert_eq!(edited.keyword.r, 0xFF, "the good line still applied");
    assert_eq!(edited.comment, base.comment, "the bad value was skipped");
    assert_eq!(edited.string, base.string, "the empty value was skipped");
}

/// Overrides land on the RESOLVED palette. With `theme = "system"` an edit
/// belongs to the light or dark palette that was actually on screen — not to a
/// palette called "system", which does not exist.
#[test]
fn overrides_are_keyed_by_the_resolved_palette() {
    let mut cfg = Config::default();
    cfg.theme = "system".into();
    cfg.theme_overrides
        .insert("dark".into(), overrides(&[("keyword", "#ABCDEF")]));

    let in_dark = theme::effective(&cfg.theme, &cfg, true);
    assert_eq!(in_dark.keyword.r, 0xAB, "dark is what system resolved to");

    let in_light = theme::effective(&cfg.theme, &cfg, false);
    assert_eq!(
        in_light.keyword,
        theme::resolve("light", false).keyword,
        "the dark edit must not follow us into light"
    );
}

/// Editing Ocean must not repaint Monokai.
#[test]
fn one_palettes_edits_do_not_reach_another() {
    let mut cfg = Config::default();
    cfg.theme_overrides
        .insert("ocean".into(), overrides(&[("keyword", "#111111")]));

    cfg.theme = "ocean".into();
    assert_eq!(theme::effective(&cfg.theme, &cfg, true).keyword.r, 0x11);

    cfg.theme = "monokai".into();
    assert_eq!(
        theme::effective(&cfg.theme, &cfg, true).keyword,
        theme::resolve("monokai", true).keyword
    );
}

/// The accent has two different controls and they are not the same request.
///
/// `highlight_color` means "make the accent this and re-derive everything
/// downstream" — selection, search, panel selection, hunk markers, and the text
/// drawn on accent. Overriding `ThemeToken::Accent` means "this exact colour,
/// leave the rest alone". Keeping both is only defensible if they measurably
/// differ.
#[test]
fn the_highlight_preference_and_an_accent_override_differ() {
    let base = theme::resolve("ocean", true);

    let mut derived = Config::default();
    derived.theme = "ocean".into();
    derived.highlight_color = "#FF2D55".into();
    let derived = theme::effective(&derived.theme, &derived, true);

    let mut exact = Config::default();
    exact.theme = "ocean".into();
    exact
        .theme_overrides
        .insert("ocean".into(), overrides(&[("accent", "#FF2D55")]));
    let exact = theme::effective(&exact.theme, &exact, true);

    assert_eq!(derived.accent, exact.accent, "both set the accent itself");
    assert_ne!(
        derived.selection_bg, base.selection_bg,
        "the highlight preference re-derives the selection"
    );
    assert_eq!(
        exact.selection_bg, base.selection_bg,
        "an accent override must touch nothing but the accent"
    );
}

/// An override is applied AFTER the highlight tint, so the last word belongs to
/// the person who picked the exact colour.
#[test]
fn an_override_wins_over_the_highlight_tint() {
    let mut cfg = Config::default();
    cfg.theme = "ocean".into();
    cfg.highlight_color = "#FF2D55".into();
    cfg.theme_overrides
        .insert("ocean".into(), overrides(&[("accent", "#00FF00")]));

    let theme = theme::effective(&cfg.theme, &cfg, true);
    assert_eq!((theme.accent.r, theme.accent.g, theme.accent.b), (0, 255, 0));
}

/// The measurement that replaces the prohibition. If this stops being honest,
/// the warning in Settings stops being worth anything.
#[test]
fn contrast_reports_the_unreadable_case() {
    let black = theme::rgb(0, 0, 0);
    let white = theme::rgb(255, 255, 255);

    assert!(
        (theme::contrast_ratio(black, white) - 21.0).abs() < 0.05,
        "black on white is the 21:1 end of the scale"
    );
    assert!(
        (theme::contrast_ratio(white, white) - 1.0).abs() < 0.01,
        "a colour against itself is 1:1"
    );
    assert!(
        theme::contrast_ratio(black, white) == theme::contrast_ratio(white, black),
        "order must not matter"
    );

    // The concrete failure the old refusal was guarding against: dark grey ink
    // on a near-black editor background.
    let ink = theme::rgb(0x33, 0x33, 0x33);
    let bg = theme::rgb(0x0F, 0x11, 0x1A);
    assert!(
        theme::contrast_ratio(ink, bg) < 3.0,
        "this is the combination Settings must warn about"
    );

    let ocean = theme::resolve("ocean", true);
    assert!(
        theme::contrast_ratio(ocean.fg, ocean.editor_bg) >= 4.5,
        "an authored palette passes the bar its editor warns by"
    );
}

/// Setting, clearing, and not leaving empty tables behind.
#[test]
fn the_panel_sets_and_clears_one_token() {
    let mut panel = SettingsPanel::new();
    assert_eq!(panel.theme_override_count("ocean"), 0);

    let action = panel.set_theme_token("ocean", ThemeToken::Keyword, "#00FF00");
    assert_eq!(action, SettingsAction::ApplyTheme);
    assert!(panel.dirty);
    assert_eq!(panel.theme_override_count("ocean"), 1);
    assert_eq!(
        panel.draft.theme_overrides["ocean"]["keyword"], "#00FF00",
        "stored uppercase with a leading #"
    );

    // Setting the same value again is not a change, so it must not dirty the
    // draft — otherwise a colour well that re-reports its value on every drag
    // frame would rewrite the config file continuously.
    panel.dirty = false;
    assert_eq!(
        panel.set_theme_token("ocean", ThemeToken::Keyword, "#00ff00"),
        SettingsAction::None
    );
    assert!(!panel.dirty);

    assert_eq!(
        panel.set_theme_token("ocean", ThemeToken::Keyword, "default"),
        SettingsAction::ApplyTheme
    );
    assert_eq!(panel.theme_override_count("ocean"), 0);
    assert!(
        !panel.draft.theme_overrides.contains_key("ocean"),
        "the last override going away must take its table with it"
    );
}

#[test]
fn the_panel_resets_a_whole_palette() {
    let mut panel = SettingsPanel::new();
    panel.set_theme_token("nord", ThemeToken::Keyword, "#111111");
    panel.set_theme_token("nord", ThemeToken::Comment, "#222222");
    panel.set_theme_token("ocean", ThemeToken::Keyword, "#333333");
    assert_eq!(panel.theme_override_count("nord"), 2);

    assert_eq!(panel.reset_theme_tokens("nord"), SettingsAction::ApplyTheme);
    assert_eq!(panel.theme_override_count("nord"), 0);
    assert_eq!(
        panel.theme_override_count("ocean"),
        1,
        "resetting one palette must not touch another"
    );
    assert_eq!(
        panel.reset_theme_tokens("nord"),
        SettingsAction::None,
        "resetting nothing is not a change"
    );
}

/// Garbage from the face is refused rather than stored.
#[test]
fn the_panel_refuses_a_value_that_is_not_a_colour() {
    let mut panel = SettingsPanel::new();
    for bad in ["#12345", "#GGGGGG", "rgb(1,2,3)", "#1234567"] {
        assert_eq!(
            panel.set_theme_token("ocean", ThemeToken::Keyword, bad),
            SettingsAction::None,
            "{bad} is not a colour"
        );
    }
    assert_eq!(panel.theme_override_count("ocean"), 0);
    assert!(!panel.dirty);
}
