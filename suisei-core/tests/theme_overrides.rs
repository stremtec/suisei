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
    // Asked of `dark`, because only the two canvas palettes take the highlight
    // tint at all — see `a_named_palette_keeps_its_own_accent`.
    let base = theme::resolve("dark", true);

    let mut derived = Config::default();
    derived.theme = "dark".into();
    derived.highlight_color = "#FF2D55".into();
    let derived = theme::effective(&derived.theme, &derived, true);

    let mut exact = Config::default();
    exact.theme = "dark".into();
    exact
        .theme_overrides
        .insert("dark".into(), overrides(&[("accent", "#FF2D55")]));
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

/// Saving MOVES the edits. The point of "Save as New Theme" is that the palette
/// you started from goes back to how its author made it, and your version lives
/// beside it — leaving the edits on both would give you two identical themes
/// and no way to see the original again.
#[test]
fn saving_a_theme_takes_the_edits_with_it() {
    let mut panel = SettingsPanel::new();
    panel.draft.theme = "catppuccin".into();
    panel.set_theme_token("catppuccin", ThemeToken::Keyword, "#FF0000");

    assert_eq!(panel.save_theme_as("Midnight", "catppuccin").as_deref(), Some("Midnight"));
    assert_eq!(panel.draft.custom_themes["Midnight"], "catppuccin");
    assert_eq!(panel.draft.theme, "Midnight", "the new theme becomes current");
    assert_eq!(panel.theme_override_count("catppuccin"), 0, "the base is clean again");
    assert_eq!(
        panel.draft.theme_overrides["Midnight"]["keyword"], "#FF0000",
        "the edits went with the new theme"
    );

    // And it resolves: base palette, then its own edits.
    let painted = theme::effective("Midnight", &panel.draft, true);
    assert_eq!(painted.keyword.r, 0xFF);
    assert_eq!(
        painted.string,
        theme::resolve("catppuccin", true).string,
        "everything unedited still comes from the base"
    );
}

/// Editing a custom theme must not reach the palette it came from.
#[test]
fn a_custom_theme_edits_on_its_own() {
    let mut panel = SettingsPanel::new();
    panel.draft.theme = "ocean".into();
    panel.set_theme_token("ocean", ThemeToken::Keyword, "#FF0000");
    panel.save_theme_as("Mine", "ocean").unwrap();

    panel.set_theme_token("Mine", ThemeToken::Comment, "#00FF00");
    assert_eq!(panel.theme_override_count("Mine"), 2);
    assert_eq!(panel.theme_override_count("ocean"), 0);

    assert_eq!(
        theme::effective("ocean", &panel.draft, true).comment,
        theme::resolve("ocean", true).comment,
        "the base palette is untouched"
    );
}

/// A base must be a built-in. Saving a custom theme from another custom theme
/// would build a chain, and deleting a link in the middle would orphan
/// everything after it.
///
/// Saving FROM a user-made theme also copies rather than moves: that theme is
/// yours, and making a second version must not empty the first.
#[test]
fn a_custom_theme_never_bases_on_another_custom_theme() {
    let mut panel = SettingsPanel::new();
    panel.set_theme_token("nord", ThemeToken::Keyword, "#111111");
    panel.save_theme_as("First", "nord").unwrap();
    panel.set_theme_token("First", ThemeToken::Comment, "#222222");
    panel.save_theme_as("Second", "First").unwrap();

    assert_eq!(
        panel.draft.custom_themes["Second"], "nord",
        "the chain is flattened to the built-in at its root"
    );
    assert_eq!(
        panel.theme_override_count("First"),
        2,
        "saving from a user-made theme copies; it must not empty the original"
    );
    assert_eq!(panel.theme_override_count("Second"), 2);
}

/// A second theme by the same name — in any capitalisation — is refused.
#[test]
fn a_saved_theme_name_is_taken_once() {
    let mut panel = SettingsPanel::new();
    panel.set_theme_token("nord", ThemeToken::Keyword, "#111111");
    assert!(panel.save_theme_as("Midnight", "nord").is_some());
    panel.set_theme_token("nord", ThemeToken::Keyword, "#222222");
    assert_eq!(
        panel.save_theme_as("midnight", "nord"),
        None,
        "two themes differing only in case are two nobody can tell apart"
    );
}

/// Names that would shadow a built-in, or name nothing at all, are refused.
#[test]
fn a_saved_theme_cannot_shadow_a_built_in() {
    let mut panel = SettingsPanel::new();
    panel.set_theme_token("ocean", ThemeToken::Keyword, "#111111");
    for bad in ["", "   ", "ocean", "Dark", "CATPPUCCIN"] {
        assert_eq!(panel.save_theme_as(bad, "ocean"), None, "{bad:?} must be refused");
    }
    assert!(panel.draft.custom_themes.is_empty());
}

/// Deleting the theme in use must leave `theme` naming something that resolves.
#[test]
fn deleting_the_theme_in_use_falls_back_to_its_base() {
    let mut panel = SettingsPanel::new();
    panel.set_theme_token("gruvbox", ThemeToken::Keyword, "#111111");
    panel.save_theme_as("Warm", "gruvbox").unwrap();
    assert_eq!(panel.draft.theme, "Warm");

    assert_eq!(panel.delete_custom_theme("Warm"), SettingsAction::ApplyTheme);
    assert_eq!(panel.draft.theme, "gruvbox");
    assert!(!panel.draft.theme_overrides.contains_key("Warm"));
    assert_eq!(
        panel.delete_custom_theme("Warm"),
        SettingsAction::None,
        "deleting nothing is not a change"
    );
}

/// Catppuccin is a real, findable palette — not just a name in a list.
#[test]
fn catppuccin_is_in_the_catalogue() {
    let t = theme::find("catppuccin").expect("catppuccin is a built-in");
    // The published Mocha base and text, so a typo in the table fails here
    // rather than looking merely "a bit off" on screen.
    assert_eq!(
        (t.editor_bg.r, t.editor_bg.g, t.editor_bg.b),
        (0x1E, 0x1E, 0x2E),
        "base #1E1E2E"
    );
    assert_eq!((t.fg.r, t.fg.g, t.fg.b), (0xCD, 0xD6, 0xF4), "text #CDD6F4");
    assert_eq!(
        (t.keyword.r, t.keyword.g, t.keyword.b),
        (0xCB, 0xA6, 0xF7),
        "mauve #CBA6F7 for keywords"
    );
    assert!(
        theme::contrast_ratio(t.fg, t.editor_bg) >= 4.5,
        "a palette shipped by us passes the bar our own editor warns by"
    );
}

/// A named palette lands whole. Its accent is part of it.
///
/// `highlight_color` is a tint for the two palettes that are a CANVAS — Light
/// and Dark are deliberately neutral and their accent is the one colour a user
/// is expected to choose. Catppuccin's mauve was picked against Catppuccin's
/// background by whoever authored it, and a leftover highlight preference
/// repainting it means choosing a theme gets you most of a theme.
#[test]
fn a_named_palette_keeps_its_own_accent() {
    let mut cfg = Config::default();
    cfg.highlight_color = "#FF2D55".into();

    for name in ["catppuccin", "ocean", "monokai", "nord", "gruvbox"] {
        cfg.theme = name.into();
        let painted = theme::effective(&cfg.theme, &cfg, true);
        let authored = theme::resolve(name, true);
        assert_eq!(
            painted.accent, authored.accent,
            "{name} must keep the accent it was authored with"
        );
        assert_eq!(
            painted.selection_bg, authored.selection_bg,
            "{name}'s selection is derived from its own accent, not the preference"
        );
    }

    // Light and Dark are canvases, and still take the tint.
    for name in ["light", "dark"] {
        cfg.theme = name.into();
        let painted = theme::effective(&cfg.theme, &cfg, name == "dark");
        assert_eq!(
            (painted.accent.r, painted.accent.g, painted.accent.b),
            (0xFF, 0x2D, 0x55),
            "{name} is a canvas — the highlight preference is what colours it"
        );
    }
}

/// An explicit per-token edit still wins on a named palette. That is a change
/// made TO this theme, not a setting left over from another one.
#[test]
fn an_override_still_reaches_a_named_palette() {
    let mut cfg = Config::default();
    cfg.theme = "catppuccin".into();
    cfg.highlight_color = "#FF2D55".into();
    cfg.theme_overrides
        .insert("catppuccin".into(), overrides(&[("accent", "#00FF00")]));

    let painted = theme::effective(&cfg.theme, &cfg, true);
    assert_eq!((painted.accent.r, painted.accent.g, painted.accent.b), (0, 255, 0));
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
