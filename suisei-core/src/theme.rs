/// A theme colour: sRGB with straight alpha.
///
/// This was `ratatui::style::Color` — a *terminal* colour type — which had two
/// costs. The engine had to recover RGB by `format!("{c:?}")` and parsing the
/// Debug string back, once per colour per frame; and a terminal palette has no
/// alpha, so chrome had to bake opaque greys where macOS composites
/// (`separator` is `white α0.10`, and stops matching the moment the surface
/// behind it changes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// Opaque colour.
pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
    Rgba { r, g, b, a: 255 }
}

/// Colour with straight alpha, for surfaces that composite over what is behind.
pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Rgba {
    Rgba { r, g, b, a }
}

impl Rgba {
    /// `0xAARRGGBB` — the packed form the face decodes.
    pub const fn argb(self) -> u32 {
        ((self.a as u32) << 24) | ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }
}

/// Syntax + chrome colors for the editor.
///
/// Prefer semantic fields (`accent`, `success`, `panel_*`, `mode_*`, `git_*`)
/// for new UI instead of hard-coded `Color::Rgb` in the TUI.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    #[allow(dead_code)]
    pub bg: Rgba,
    pub fg: Rgba,
    pub keyword: Rgba,
    pub string: Rgba,
    pub comment: Rgba,
    pub number: Rgba,
    pub type_name: Rgba,
    pub function: Rgba,
    pub macro_name: Rgba,
    pub namespace: Rgba,
    pub parameter: Rgba,
    pub property: Rgba,
    pub constant: Rgba,
    pub operator: Rgba,
    pub punctuation: Rgba,
    pub line_no: Rgba,
    /// Wash behind the row the caret is on.
    pub current_line: Rgba,
    /// Tabs and trailing spaces, when they are shown.
    pub invisibles: Rgba,
    pub editor_bg: Rgba,
    /// The 3D workbench's stage.
    ///
    /// White in every palette that ships, on purpose. A model arrives with
    /// its own materials and its own lighting, and the one thing a viewer
    /// must not do is change what they look like — a dark stage makes an
    /// unlit mesh read as a silhouette and a light one as a form. It is a
    /// theme token rather than a constant so a theme CAN move it; it is not
    /// derived from `editor_bg` so that switching to a dark theme does not
    /// silently move it.
    pub model_bg: Rgba,
    pub status_bg: Rgba,
    pub status_fg: Rgba,
    pub border: Rgba,
    pub selection_bg: Rgba,
    pub search_bg: Rgba,
    pub mode_xlc: Rgba,
    pub completion_bg: Rgba,
    pub completion_selected: Rgba,
    pub completion_border: Rgba,
    pub explorer_bg: Rgba,
    pub explorer_fg: Rgba,
    pub explorer_dir: Rgba,
    pub explorer_selected: Rgba,
    pub terminal_bg: Rgba,
    #[allow(dead_code)]
    pub terminal_fg: Rgba,
    #[allow(dead_code)]
    pub terminal_prompt: Rgba,
    pub xlc_bg: Rgba,
    pub xlc_fg: Rgba,
    pub xlc_prompt: Rgba,
    pub xlc_border: Rgba,
    pub cursor: Rgba,
    // ── Semantic UI (feature surfaces) ──
    /// Primary interactive accent (chips, focus borders, links).
    pub accent: Rgba,
    /// Text drawn on `accent` background.
    pub accent_fg: Rgba,
    /// Secondary / dim text (hints, meta).
    pub muted: Rgba,
    /// Positive / staged / success.
    pub success: Rgba,
    /// Warning / modified.
    pub warning: Rgba,
    /// Error / deleted / danger.
    pub error: Rgba,
    /// Popup / context-menu / palette body.
    pub panel_bg: Rgba,
    pub panel_border: Rgba,
    /// Selected row inside panels/lists.
    pub panel_sel_bg: Rgba,
    pub panel_sel_fg: Rgba,
    /// Status-bar mode chips for non-vim modes.
    pub mode_git: Rgba,
    pub mode_term: Rgba,
    pub mode_preview: Rgba,
    pub mode_settings: Rgba,
    pub mode_search: Rgba,
    pub mode_find: Rgba,
    /// Diff line backgrounds + hunk header.
    pub git_add_bg: Rgba,
    pub git_del_bg: Rgba,
    pub git_hunk: Rgba,
}

static THEMES: &[Theme] = &[
    LIGHT, DARK, CATPPUCCIN, OCEAN, MONOKAI, NORD, SOLARIZED, GRUVBOX, EVERFOREST, SAKURA,
    NEWSPAPER, MONO, MONO_DARK,
];

// ── LIGHT — production default, quiet neutral canvas + Suisei blue ──
//
// Pure white plus system-blue selections made every selected row louder than
// the source text and produced glare across a full editor window. The default
// now uses a near-white canvas, a restrained cobalt accent, and syntax hues
// separated by role rather than by maximum saturation.
pub static LIGHT: Theme = Theme {
    name: "light",
    // The editor's own surface, not a shade under it. Eleven of the thirteen
    // palettes set these two the same and show nothing where the window floor
    // meets the editor; light and dark were the only two that split them, by
    // six and nine units — too small to read as intentional, large enough to
    // see as a rendering fault, and visible anywhere the two surfaces touch:
    // beside the inspector, under the transparent titlebar, along the status
    // bar. Every one of those was a separate hunt for a seam the palette was
    // creating on purpose.
    bg: rgb(252, 253, 254),
    fg: rgb(35, 40, 47),
    keyword: rgb(137, 46, 147),
    string: rgb(177, 45, 36),
    comment: rgb(91, 103, 116),
    number: rgb(0, 82, 174),
    type_name: rgb(0, 94, 112),
    function: rgb(0, 112, 102),
    macro_name: rgb(166, 74, 0),
    namespace: rgb(51, 91, 160),
    parameter: rgb(73, 80, 87),
    property: rgb(34, 101, 147),
    constant: rgb(112, 57, 168),
    operator: rgb(48, 54, 61),
    punctuation: rgb(106, 115, 125),
    line_no: rgb(143, 151, 160),
    current_line: rgba(35, 40, 47, 16),
    invisibles: rgb(192, 197, 202),
    editor_bg: rgb(252, 253, 254),
    model_bg: rgb(255, 255, 255),
    status_bg: rgb(243, 246, 249),
    status_fg: rgb(75, 82, 90),
    border: rgb(219, 224, 230),
    selection_bg: rgb(195, 220, 247),
    search_bg: rgb(255, 228, 142),
    mode_xlc: rgb(11, 110, 222),
    completion_bg: rgb(255, 255, 255),
    completion_selected: rgb(11, 110, 222),
    completion_border: rgb(219, 224, 230),
    explorer_bg: rgb(247, 249, 251),
    explorer_fg: rgb(64, 70, 78),
    explorer_dir: rgb(11, 110, 222),
    explorer_selected: rgb(224, 237, 250),
    terminal_bg: rgb(248, 250, 252),
    terminal_fg: rgb(35, 40, 47),
    terminal_prompt: rgb(11, 110, 222),
    xlc_bg: rgb(248, 250, 252),
    xlc_fg: rgb(35, 40, 47),
    xlc_prompt: rgb(11, 110, 222),
    xlc_border: rgb(219, 224, 230),
    cursor: rgb(11, 110, 222),
    accent: rgb(11, 110, 222),
    accent_fg: rgb(255, 255, 255),
    muted: rgb(119, 127, 137),
    success: rgb(38, 139, 76),
    warning: rgb(201, 111, 0),
    error: rgb(210, 45, 42),
    panel_bg: rgb(255, 255, 255),
    panel_border: rgb(219, 224, 230),
    panel_sel_bg: rgb(231, 240, 250),
    panel_sel_fg: rgb(27, 32, 38),
    mode_git: rgb(38, 139, 76),
    mode_term: rgb(38, 139, 76),
    mode_preview: rgb(91, 79, 196),
    mode_settings: rgb(119, 127, 137),
    mode_search: rgb(201, 111, 0),
    mode_find: rgb(11, 110, 222),
    git_add_bg: rgb(233, 248, 239),
    git_del_bg: rgb(253, 237, 235),
    git_hunk: rgb(11, 110, 222),
};

// ── DARK — production default, warm-neutral near-black, elevation by value ──
/// Default dark theme: a **warm-neutral near-black** with clearly separated
/// elevation steps.
///
/// Two earlier problems, both fixed here. First it read *blue* — every surface
/// had its blue channel highest. Then, neutralised to a flat `rgb(16,16,16)`
/// it read as one dead pure-black slab, because the floor, editor, sidebar and
/// terminal all sat within six levels of each other. The fix is a real ramp
/// with a **faint warm cast** (red ≥ green ≥ blue, by 1–3) so the black reads
/// expensive rather than grey or cold:
///   L0 floor   rgb(18,17,16)  window channel · status · terminal grid
///   nav        rgb(23,22,21)  file explorer
///   L1 surface rgb(27,26,24)  editor — the hero surface, clearly lifted
///   L2 raised  rgb(38,37,34)  popovers · completion · panels
/// Text is crisp neutral near-white (`rgb(233,233,233)`), separators a faint
/// `white α0.08` hairline, and elevation reads by brightness alone.
///
/// Accents/status stay on Apple's dark **system colours** (systemBlue #0A84FF,
/// systemGreen #30D158, systemOrange #FF9F0A, systemRed #FF453A) — blue lives
/// only in accents and selection now, never in a surface. Syntax hues unchanged.
pub static DARK: Theme = Theme {
    name: "dark",
    // See `light` — its floor is its editor surface for the same reason.
    bg: rgb(27, 26, 24),
    fg: rgb(233, 233, 233),
    keyword: rgb(207, 121, 207),
    string: rgb(219, 143, 121),
    comment: rgb(119, 119, 119),
    number: rgb(181, 206, 168),
    type_name: rgb(78, 201, 176),
    function: rgb(105, 164, 255),
    macro_name: rgb(209, 154, 102),
    namespace: rgb(86, 182, 194),
    parameter: rgb(207, 207, 207),
    property: rgb(126, 200, 227),
    constant: rgb(220, 210, 134),
    operator: rgb(233, 233, 233),
    punctuation: rgb(144, 144, 144),
    line_no: rgb(96, 96, 96),
    current_line: rgba(233, 233, 233, 16),
    invisibles: rgb(65, 64, 64),
    editor_bg: rgb(27, 26, 24),
    model_bg: rgb(255, 255, 255),
    status_bg: rgb(18, 17, 16),
    status_fg: rgb(164, 164, 164),
    border: rgba(255, 255, 255, 20),
    selection_bg: rgb(28, 58, 102),
    search_bg: rgb(92, 78, 30),
    mode_xlc: rgb(10, 132, 255),
    completion_bg: rgb(38, 37, 34),
    completion_selected: rgb(10, 132, 255),
    completion_border: rgba(255, 255, 255, 20),
    explorer_bg: rgb(23, 22, 21),
    explorer_fg: rgb(191, 191, 191),
    explorer_dir: rgb(10, 132, 255),
    explorer_selected: rgb(28, 58, 102),
    terminal_bg: rgb(18, 17, 16),
    terminal_fg: rgb(233, 233, 233),
    terminal_prompt: rgb(10, 132, 255),
    xlc_bg: rgb(38, 37, 34),
    xlc_fg: rgb(233, 233, 233),
    xlc_prompt: rgb(10, 132, 255),
    xlc_border: rgba(255, 255, 255, 20),
    cursor: rgb(10, 132, 255),
    accent: rgb(10, 132, 255),
    accent_fg: rgb(255, 255, 255),
    muted: rgb(128, 128, 128),
    success: rgb(48, 209, 88),
    warning: rgb(255, 159, 10),
    error: rgb(255, 69, 58),
    panel_bg: rgb(38, 37, 34),
    panel_border: rgba(255, 255, 255, 20),
    panel_sel_bg: rgb(28, 58, 102),
    panel_sel_fg: rgb(240, 240, 240),
    mode_git: rgb(48, 209, 88),
    mode_term: rgb(48, 209, 88),
    mode_preview: rgb(94, 92, 230),
    mode_settings: rgb(142, 144, 150),
    mode_search: rgb(255, 159, 10),
    mode_find: rgb(10, 132, 255),
    git_add_bg: rgb(22, 46, 34),
    git_del_bg: rgb(54, 32, 36),
    git_hunk: rgb(10, 132, 255),
};

pub fn all_themes() -> &'static [Theme] {
    THEMES
}

pub fn find(name: &str) -> Option<&'static Theme> {
    THEMES
        .iter()
        .find(|t| t.name.to_lowercase() == name.to_lowercase())
}

// ── OCEAN ──
pub static OCEAN: Theme = Theme {
    name: "ocean",
    bg: rgb(15, 17, 26),
    fg: rgb(200, 210, 220),
    keyword: rgb(0, 220, 255),
    string: rgb(150, 230, 180),
    comment: rgb(96, 108, 122),
    number: rgb(255, 180, 130),
    type_name: rgb(100, 200, 255),
    function: rgb(255, 220, 120),
    macro_name: rgb(255, 160, 220),
    namespace: rgb(140, 180, 255),
    parameter: rgb(200, 180, 140),
    property: rgb(130, 210, 200),
    constant: rgb(255, 180, 100),
    operator: rgb(180, 190, 200),
    punctuation: rgb(120, 130, 150),
    line_no: rgb(82, 92, 114),
    current_line: rgba(200, 210, 220, 16),
    invisibles: rgb(52, 58, 74),
    editor_bg: rgb(15, 17, 26),
    model_bg: rgb(255, 255, 255),
    status_bg: rgb(10, 12, 20),
    status_fg: rgb(180, 190, 200),
    border: rgb(58, 66, 88),
    selection_bg: rgb(52, 66, 110),
    search_bg: rgb(104, 88, 26),
    mode_xlc: rgb(0, 180, 200),
    completion_bg: rgb(25, 28, 38),
    completion_selected: rgb(0, 220, 255),
    completion_border: rgb(0, 220, 255),
    explorer_bg: rgb(12, 14, 22),
    explorer_fg: rgb(190, 200, 210),
    explorer_dir: rgb(100, 150, 255),
    explorer_selected: rgb(0, 220, 255),
    terminal_bg: rgb(8, 12, 8),
    terminal_fg: rgb(180, 255, 180),
    terminal_prompt: rgb(48, 209, 88),
    xlc_bg: rgb(18, 20, 28),
    xlc_fg: rgb(152, 152, 157),
    xlc_prompt: rgb(48, 209, 88),
    xlc_border: rgb(0, 220, 255),
    cursor: rgb(200, 200, 220),
    accent: rgb(70, 130, 200),
    accent_fg: rgb(0, 0, 0),
    muted: rgb(100, 110, 130),
    success: rgb(100, 200, 140),
    warning: rgb(220, 180, 80),
    error: rgb(240, 120, 120),
    panel_bg: rgb(25, 28, 38),
    panel_border: rgb(70, 130, 200),
    panel_sel_bg: rgb(45, 70, 100),
    panel_sel_fg: rgb(230, 235, 255),
    mode_git: rgb(80, 200, 140),
    mode_term: rgb(80, 200, 120),
    mode_preview: rgb(140, 190, 255),
    mode_settings: rgb(160, 150, 220),
    mode_search: rgb(230, 200, 80),
    mode_find: rgb(100, 180, 255),
    git_add_bg: rgb(20, 40, 28),
    git_del_bg: rgb(45, 22, 24),
    git_hunk: rgb(100, 180, 255),
};

// ── CATPPUCCIN — Mocha, from the published palette ──
//
// `bg` is base, the same as `editor_bg`, and that is deliberate. Nine of the
// palettes here set the two identically; the window floor IS the primary
// surface, and mantle's job is the SECONDARY ones — the navigator, the status
// bar, panels — which is where it is used below and what upstream Catppuccin
// does with it too.
//
// It was mantle, and that is the one palette in this file where the difference
// is visible: Light and Dark also separate them, by 6 and 9 levels, but in
// near-white and near-black where the eye does not read the step. The same 9
// levels in a mid-value violet does read, as a seam along every edge where the
// titlebar and the shell meet the document.
//
// The named colours are Catppuccin Mocha as specified, not eyeballed:
//   base #1E1E2E  mantle #181825  crust #11111B  surface0 #313244
//   surface1 #45475A  overlay0 #6C7086  text #CDD6F4  subtext0 #A6ADC8
//   rosewater #F5E0DC  flamingo #F2CDCD  pink #F5C2E7  mauve #CBA6F7
//   red #F38BA8  peach #FAB387  yellow #F9E2AF  green #A6E3A1
//   teal #94E2D5  sky #89DCEB  sapphire #74C7EC  blue #89B4FA
//   lavender #B4BEFE
// Role assignment follows the upstream syntax mapping: mauve for keywords,
// green for strings, peach for numbers, yellow for types, blue for functions.
pub static CATPPUCCIN: Theme = Theme {
    name: "catppuccin",
    bg: rgb(30, 30, 46),
    fg: rgb(205, 214, 244),
    keyword: rgb(203, 166, 247),
    string: rgb(166, 227, 161),
    comment: rgb(108, 112, 134),
    number: rgb(250, 179, 135),
    type_name: rgb(249, 226, 175),
    function: rgb(137, 180, 250),
    macro_name: rgb(245, 194, 231),
    namespace: rgb(180, 190, 254),
    parameter: rgb(235, 160, 172),
    property: rgb(148, 226, 213),
    constant: rgb(250, 179, 135),
    operator: rgb(137, 220, 235),
    punctuation: rgb(147, 153, 178),
    line_no: rgb(88, 91, 112),
    current_line: rgba(205, 214, 244, 16),
    invisibles: rgb(58, 60, 78),
    editor_bg: rgb(30, 30, 46),
    model_bg: rgb(255, 255, 255),
    status_bg: rgb(24, 24, 37),
    status_fg: rgb(166, 173, 200),
    border: rgb(49, 50, 68),
    selection_bg: rgb(69, 71, 90),
    search_bg: rgb(85, 79, 78),
    mode_xlc: rgb(203, 166, 247),
    completion_bg: rgb(24, 24, 37),
    completion_selected: rgb(203, 166, 247),
    completion_border: rgb(49, 50, 68),
    explorer_bg: rgb(24, 24, 37),
    explorer_fg: rgb(186, 194, 222),
    explorer_dir: rgb(203, 166, 247),
    explorer_selected: rgb(49, 50, 68),
    terminal_bg: rgb(17, 17, 27),
    terminal_fg: rgb(205, 214, 244),
    terminal_prompt: rgb(166, 227, 161),
    xlc_bg: rgb(24, 24, 37),
    xlc_fg: rgb(166, 173, 200),
    xlc_prompt: rgb(166, 227, 161),
    xlc_border: rgb(203, 166, 247),
    cursor: rgb(245, 224, 220),
    accent: rgb(203, 166, 247),
    accent_fg: rgb(17, 17, 27),
    muted: rgb(127, 132, 156),
    success: rgb(166, 227, 161),
    warning: rgb(249, 226, 175),
    error: rgb(243, 139, 168),
    panel_bg: rgb(24, 24, 37),
    panel_border: rgb(49, 50, 68),
    panel_sel_bg: rgb(49, 50, 68),
    panel_sel_fg: rgb(205, 214, 244),
    mode_git: rgb(166, 227, 161),
    mode_term: rgb(148, 226, 213),
    mode_preview: rgb(180, 190, 254),
    mode_settings: rgb(203, 166, 247),
    mode_search: rgb(249, 226, 175),
    mode_find: rgb(203, 166, 247),
    git_add_bg: rgb(32, 45, 40),
    git_del_bg: rgb(52, 34, 42),
    git_hunk: rgb(203, 166, 247),
};

// ── MONOKAI ──
pub static MONOKAI: Theme = Theme {
    name: "monokai",
    bg: rgb(39, 40, 34),
    fg: rgb(248, 248, 242),
    keyword: rgb(249, 38, 114),
    string: rgb(230, 219, 116),
    comment: rgb(117, 113, 94),
    number: rgb(174, 129, 255),
    type_name: rgb(166, 226, 46),
    function: rgb(166, 226, 46),
    macro_name: rgb(102, 217, 239),
    namespace: rgb(174, 129, 255),
    parameter: rgb(253, 151, 31),
    property: rgb(248, 248, 242),
    constant: rgb(174, 129, 255),
    operator: rgb(249, 38, 114),
    punctuation: rgb(117, 113, 94),
    line_no: rgb(80, 80, 70),
    current_line: rgba(248, 248, 242, 16),
    invisibles: rgb(62, 62, 54),
    editor_bg: rgb(39, 40, 34),
    model_bg: rgb(255, 255, 255),
    status_bg: rgb(30, 31, 26),
    status_fg: rgb(200, 200, 195),
    border: rgb(70, 70, 60),
    selection_bg: rgb(70, 65, 55),
    search_bg: rgb(100, 90, 40),
    mode_xlc: rgb(249, 38, 114),
    completion_bg: rgb(45, 46, 40),
    completion_selected: rgb(249, 38, 114),
    completion_border: rgb(249, 38, 114),
    explorer_bg: rgb(32, 33, 28),
    explorer_fg: rgb(240, 240, 235),
    explorer_dir: rgb(102, 217, 239),
    explorer_selected: rgb(249, 38, 114),
    terminal_bg: rgb(30, 31, 26),
    terminal_fg: rgb(230, 219, 116),
    terminal_prompt: rgb(166, 226, 46),
    xlc_bg: rgb(35, 36, 30),
    xlc_fg: rgb(180, 180, 170),
    xlc_prompt: rgb(166, 226, 46),
    xlc_border: rgb(249, 38, 114),
    cursor: rgb(248, 248, 242),
    accent: rgb(249, 38, 114),
    accent_fg: rgb(0, 0, 0),
    muted: rgb(117, 113, 94),
    success: rgb(166, 226, 46),
    warning: rgb(253, 151, 31),
    error: rgb(249, 38, 114),
    panel_bg: rgb(45, 46, 40),
    panel_border: rgb(249, 38, 114),
    panel_sel_bg: rgb(70, 45, 55),
    panel_sel_fg: rgb(248, 248, 242),
    mode_git: rgb(166, 226, 46),
    mode_term: rgb(166, 226, 46),
    mode_preview: rgb(102, 217, 239),
    mode_settings: rgb(174, 129, 255),
    mode_search: rgb(230, 219, 116),
    mode_find: rgb(102, 217, 239),
    git_add_bg: rgb(40, 50, 30),
    git_del_bg: rgb(55, 30, 35),
    git_hunk: rgb(102, 217, 239),
};

// ── NORD ──
pub static NORD: Theme = Theme {
    name: "nord",
    bg: rgb(46, 52, 64),
    fg: rgb(216, 222, 233),
    keyword: rgb(129, 161, 193),
    string: rgb(163, 190, 140),
    comment: rgb(97, 110, 124),
    number: rgb(180, 142, 173),
    type_name: rgb(143, 188, 187),
    function: rgb(136, 192, 208),
    macro_name: rgb(235, 203, 139),
    namespace: rgb(129, 161, 193),
    parameter: rgb(208, 135, 112),
    property: rgb(216, 222, 233),
    constant: rgb(180, 142, 173),
    operator: rgb(129, 161, 193),
    punctuation: rgb(76, 86, 106),
    line_no: rgb(76, 86, 106),
    current_line: rgba(216, 222, 233, 16),
    invisibles: rgb(62, 71, 87),
    editor_bg: rgb(46, 52, 64),
    model_bg: rgb(255, 255, 255),
    status_bg: rgb(36, 41, 51),
    status_fg: rgb(200, 207, 218),
    border: rgb(67, 76, 94),
    selection_bg: rgb(60, 68, 88),
    search_bg: rgb(100, 90, 50),
    mode_xlc: rgb(136, 192, 208),
    completion_bg: rgb(52, 58, 70),
    completion_selected: rgb(129, 161, 193),
    completion_border: rgb(129, 161, 193),
    explorer_bg: rgb(40, 45, 56),
    explorer_fg: rgb(210, 216, 228),
    explorer_dir: rgb(94, 129, 172),
    explorer_selected: rgb(129, 161, 193),
    terminal_bg: rgb(35, 40, 50),
    terminal_fg: rgb(163, 190, 140),
    terminal_prompt: rgb(143, 188, 187),
    xlc_bg: rgb(42, 48, 58),
    xlc_fg: rgb(180, 187, 200),
    xlc_prompt: rgb(163, 190, 140),
    xlc_border: rgb(129, 161, 193),
    cursor: rgb(216, 222, 233),
    accent: rgb(129, 161, 193),
    accent_fg: rgb(0, 0, 0),
    muted: rgb(97, 110, 124),
    success: rgb(163, 190, 140),
    warning: rgb(235, 203, 139),
    error: rgb(191, 97, 106),
    panel_bg: rgb(52, 58, 70),
    panel_border: rgb(129, 161, 193),
    panel_sel_bg: rgb(67, 76, 94),
    panel_sel_fg: rgb(236, 239, 244),
    mode_git: rgb(163, 190, 140),
    mode_term: rgb(143, 188, 187),
    mode_preview: rgb(136, 192, 208),
    mode_settings: rgb(180, 142, 173),
    mode_search: rgb(235, 203, 139),
    mode_find: rgb(94, 129, 172),
    git_add_bg: rgb(40, 55, 48),
    git_del_bg: rgb(55, 40, 45),
    git_hunk: rgb(136, 192, 208),
};

// ── SOLARIZED ──
pub static SOLARIZED: Theme = Theme {
    name: "solarized",
    bg: rgb(0, 43, 54),
    fg: rgb(131, 148, 150),
    keyword: rgb(38, 139, 210),
    string: rgb(42, 161, 152),
    comment: rgb(88, 110, 117),
    number: rgb(203, 75, 22),
    type_name: rgb(181, 137, 0),
    function: rgb(38, 139, 210),
    macro_name: rgb(211, 54, 130),
    namespace: rgb(42, 161, 152),
    parameter: rgb(203, 75, 22),
    property: rgb(131, 148, 150),
    constant: rgb(203, 75, 22),
    operator: rgb(38, 139, 210),
    punctuation: rgb(88, 110, 117),
    line_no: rgb(50, 75, 85),
    current_line: rgba(131, 148, 150, 16),
    invisibles: rgb(28, 61, 71),
    editor_bg: rgb(0, 43, 54),
    model_bg: rgb(255, 255, 255),
    status_bg: rgb(0, 35, 44),
    status_fg: rgb(120, 135, 138),
    border: rgb(0, 55, 70),
    selection_bg: rgb(7, 54, 66),
    search_bg: rgb(80, 60, 20),
    mode_xlc: rgb(42, 161, 152),
    completion_bg: rgb(5, 48, 60),
    completion_selected: rgb(38, 139, 210),
    completion_border: rgb(38, 139, 210),
    explorer_bg: rgb(0, 38, 48),
    explorer_fg: rgb(125, 140, 143),
    explorer_dir: rgb(38, 139, 210),
    explorer_selected: rgb(133, 153, 0),
    terminal_bg: rgb(0, 35, 44),
    terminal_fg: rgb(133, 153, 0),
    terminal_prompt: rgb(42, 161, 152),
    xlc_bg: rgb(0, 40, 50),
    xlc_fg: rgb(110, 125, 128),
    xlc_prompt: rgb(133, 153, 0),
    xlc_border: rgb(38, 139, 210),
    cursor: rgb(131, 148, 150),
    accent: rgb(38, 139, 210),
    accent_fg: rgb(0, 43, 54),
    muted: rgb(88, 110, 117),
    success: rgb(133, 153, 0),
    warning: rgb(181, 137, 0),
    error: rgb(220, 50, 47),
    panel_bg: rgb(5, 48, 60),
    panel_border: rgb(38, 139, 210),
    panel_sel_bg: rgb(7, 54, 66),
    panel_sel_fg: rgb(253, 246, 227),
    mode_git: rgb(133, 153, 0),
    mode_term: rgb(42, 161, 152),
    mode_preview: rgb(38, 139, 210),
    mode_settings: rgb(211, 54, 130),
    mode_search: rgb(181, 137, 0),
    mode_find: rgb(38, 139, 210),
    git_add_bg: rgb(0, 50, 40),
    git_del_bg: rgb(50, 25, 25),
    git_hunk: rgb(38, 139, 210),
};

// ── GRUVBOX ──
pub static GRUVBOX: Theme = Theme {
    name: "gruvbox",
    bg: rgb(40, 40, 40),
    fg: rgb(235, 219, 178),
    keyword: rgb(211, 134, 155),
    string: rgb(184, 187, 38),
    comment: rgb(146, 131, 116),
    number: rgb(177, 98, 134),
    type_name: rgb(142, 192, 124),
    function: rgb(250, 189, 47),
    macro_name: rgb(254, 128, 25),
    namespace: rgb(69, 133, 136),
    parameter: rgb(211, 134, 155),
    property: rgb(235, 219, 178),
    constant: rgb(211, 134, 155),
    operator: rgb(254, 128, 25),
    punctuation: rgb(146, 131, 116),
    line_no: rgb(80, 73, 64),
    current_line: rgba(235, 219, 178, 16),
    invisibles: rgb(62, 58, 53),
    editor_bg: rgb(40, 40, 40),
    model_bg: rgb(255, 255, 255),
    status_bg: rgb(32, 32, 32),
    status_fg: rgb(220, 205, 165),
    border: rgb(60, 56, 50),
    selection_bg: rgb(60, 56, 50),
    search_bg: rgb(90, 80, 40),
    mode_xlc: rgb(131, 165, 152),
    completion_bg: rgb(48, 48, 48),
    completion_selected: rgb(254, 128, 25),
    completion_border: rgb(254, 128, 25),
    explorer_bg: rgb(34, 34, 34),
    explorer_fg: rgb(228, 212, 170),
    explorer_dir: rgb(69, 133, 136),
    explorer_selected: rgb(254, 128, 25),
    terminal_bg: rgb(30, 30, 30),
    terminal_fg: rgb(184, 187, 38),
    terminal_prompt: rgb(142, 192, 124),
    xlc_bg: rgb(36, 36, 36),
    xlc_fg: rgb(190, 178, 150),
    xlc_prompt: rgb(184, 187, 38),
    xlc_border: rgb(254, 128, 25),
    cursor: rgb(235, 219, 178),
    accent: rgb(254, 128, 25),
    accent_fg: rgb(0, 0, 0),
    muted: rgb(146, 131, 116),
    success: rgb(142, 192, 124),
    warning: rgb(250, 189, 47),
    error: rgb(251, 73, 52),
    panel_bg: rgb(48, 48, 48),
    panel_border: rgb(254, 128, 25),
    panel_sel_bg: rgb(60, 56, 50),
    panel_sel_fg: rgb(251, 241, 199),
    mode_git: rgb(142, 192, 124),
    mode_term: rgb(184, 187, 38),
    mode_preview: rgb(69, 133, 136),
    mode_settings: rgb(211, 134, 155),
    mode_search: rgb(250, 189, 47),
    mode_find: rgb(69, 133, 136),
    git_add_bg: rgb(35, 45, 30),
    git_del_bg: rgb(50, 30, 28),
    git_hunk: rgb(131, 165, 152),
};

// ── EVERFOREST ──
pub static EVERFOREST: Theme = Theme {
    name: "everforest",
    bg: rgb(39, 46, 44),
    fg: rgb(211, 198, 170),
    keyword: rgb(230, 126, 128),
    string: rgb(167, 192, 128),
    comment: rgb(134, 140, 122),
    number: rgb(223, 164, 126),
    type_name: rgb(130, 202, 157),
    function: rgb(219, 188, 127),
    macro_name: rgb(214, 153, 182),
    namespace: rgb(130, 170, 162),
    parameter: rgb(223, 164, 126),
    property: rgb(211, 198, 170),
    constant: rgb(230, 126, 128),
    operator: rgb(131, 180, 175),
    punctuation: rgb(134, 140, 122),
    line_no: rgb(75, 84, 68),
    current_line: rgba(211, 198, 170, 16),
    invisibles: rgb(59, 67, 57),
    editor_bg: rgb(39, 46, 44),
    model_bg: rgb(255, 255, 255),
    status_bg: rgb(31, 37, 35),
    status_fg: rgb(200, 188, 160),
    border: rgb(58, 66, 60),
    selection_bg: rgb(55, 64, 58),
    search_bg: rgb(90, 80, 40),
    mode_xlc: rgb(131, 180, 175),
    completion_bg: rgb(46, 52, 50),
    completion_selected: rgb(230, 126, 128),
    completion_border: rgb(230, 126, 128),
    explorer_bg: rgb(33, 40, 38),
    explorer_fg: rgb(205, 193, 165),
    explorer_dir: rgb(130, 170, 162),
    explorer_selected: rgb(230, 126, 128),
    terminal_bg: rgb(28, 35, 33),
    terminal_fg: rgb(167, 192, 128),
    terminal_prompt: rgb(130, 202, 157),
    xlc_bg: rgb(35, 41, 39),
    xlc_fg: rgb(185, 173, 148),
    xlc_prompt: rgb(167, 192, 128),
    xlc_border: rgb(230, 126, 128),
    cursor: rgb(211, 198, 170),
    accent: rgb(130, 170, 162),
    accent_fg: rgb(0, 0, 0),
    muted: rgb(134, 140, 122),
    success: rgb(167, 192, 128),
    warning: rgb(219, 188, 127),
    error: rgb(230, 126, 128),
    panel_bg: rgb(46, 52, 50),
    panel_border: rgb(130, 170, 162),
    panel_sel_bg: rgb(55, 64, 58),
    panel_sel_fg: rgb(230, 220, 195),
    mode_git: rgb(167, 192, 128),
    mode_term: rgb(130, 202, 157),
    mode_preview: rgb(131, 180, 175),
    mode_settings: rgb(214, 153, 182),
    mode_search: rgb(219, 188, 127),
    mode_find: rgb(130, 170, 162),
    git_add_bg: rgb(35, 48, 40),
    git_del_bg: rgb(50, 35, 35),
    git_hunk: rgb(131, 180, 175),
};

// ── SAKURA ──
pub static SAKURA: Theme = Theme {
    name: "sakura",
    bg: rgb(255, 240, 245),
    fg: rgb(60, 20, 40),
    keyword: rgb(190, 60, 100),
    string: rgb(100, 160, 120),
    comment: rgb(200, 180, 190),
    number: rgb(180, 100, 80),
    type_name: rgb(160, 100, 140),
    function: rgb(180, 80, 120),
    macro_name: rgb(140, 100, 160),
    namespace: rgb(200, 130, 160),
    parameter: rgb(180, 100, 80),
    property: rgb(60, 20, 40),
    constant: rgb(190, 60, 100),
    operator: rgb(190, 60, 100),
    punctuation: rgb(200, 180, 190),
    line_no: rgb(200, 170, 185),
    current_line: rgba(60, 20, 40, 16),
    invisibles: rgb(225, 202, 212),
    editor_bg: rgb(255, 240, 245),
    model_bg: rgb(255, 255, 255),
    status_bg: rgb(255, 225, 235),
    status_fg: rgb(60, 20, 40),
    border: rgb(240, 200, 215),
    selection_bg: rgb(255, 210, 225),
    search_bg: rgb(255, 230, 200),
    mode_xlc: rgb(180, 140, 160),
    completion_bg: rgb(255, 235, 242),
    completion_selected: rgb(219, 112, 147),
    completion_border: rgb(219, 112, 147),
    explorer_bg: rgb(255, 230, 238),
    explorer_fg: rgb(60, 20, 40),
    explorer_dir: rgb(200, 130, 160),
    explorer_selected: rgb(219, 112, 147),
    terminal_bg: rgb(255, 235, 242),
    terminal_fg: rgb(60, 20, 40),
    terminal_prompt: rgb(219, 112, 147),
    xlc_bg: rgb(255, 235, 242),
    xlc_fg: rgb(60, 20, 40),
    xlc_prompt: rgb(219, 112, 147),
    xlc_border: rgb(219, 112, 147),
    cursor: rgb(136, 34, 56),
    accent: rgb(219, 112, 147),
    accent_fg: rgb(255, 255, 255),
    muted: rgb(180, 150, 165),
    success: rgb(100, 160, 120),
    warning: rgb(200, 140, 80),
    error: rgb(200, 60, 80),
    panel_bg: rgb(255, 235, 242),
    panel_border: rgb(219, 112, 147),
    panel_sel_bg: rgb(255, 210, 225),
    panel_sel_fg: rgb(60, 20, 40),
    mode_git: rgb(150, 190, 140),
    mode_term: rgb(150, 190, 140),
    mode_preview: rgb(200, 130, 160),
    mode_settings: rgb(180, 140, 160),
    mode_search: rgb(200, 140, 80),
    mode_find: rgb(219, 112, 147),
    git_add_bg: rgb(230, 245, 235),
    git_del_bg: rgb(255, 230, 235),
    git_hunk: rgb(200, 130, 160),
};

// ── NEWSPAPER ──
pub static NEWSPAPER: Theme = Theme {
    name: "newspaper",
    bg: rgb(255, 243, 229),
    fg: rgb(125, 130, 172),
    keyword: rgb(125, 130, 172),
    string: rgb(140, 145, 185),
    comment: rgb(190, 185, 195),
    number: rgb(150, 140, 175),
    type_name: rgb(172, 176, 214),
    function: rgb(100, 110, 160),
    macro_name: rgb(140, 120, 170),
    namespace: rgb(125, 130, 172),
    parameter: rgb(150, 140, 175),
    property: rgb(125, 130, 172),
    constant: rgb(140, 145, 185),
    operator: rgb(125, 130, 172),
    punctuation: rgb(190, 185, 195),
    line_no: rgb(200, 195, 200),
    current_line: rgba(125, 130, 172, 16),
    invisibles: rgb(225, 217, 213),
    editor_bg: rgb(255, 243, 229),
    model_bg: rgb(255, 255, 255),
    status_bg: rgb(254, 249, 244),
    status_fg: rgb(125, 130, 172),
    border: rgb(228, 223, 225),
    selection_bg: rgb(228, 223, 225),
    search_bg: rgb(220, 215, 210),
    mode_xlc: rgb(172, 176, 214),
    completion_bg: rgb(254, 249, 244),
    completion_selected: rgb(125, 130, 172),
    completion_border: rgb(172, 176, 214),
    explorer_bg: rgb(254, 249, 244),
    explorer_fg: rgb(125, 130, 172),
    explorer_dir: rgb(125, 130, 172),
    explorer_selected: rgb(172, 176, 214),
    terminal_bg: rgb(254, 249, 244),
    terminal_fg: rgb(125, 130, 172),
    terminal_prompt: rgb(172, 176, 214),
    xlc_bg: rgb(254, 249, 244),
    xlc_fg: rgb(125, 130, 172),
    xlc_prompt: rgb(172, 176, 214),
    xlc_border: rgb(228, 223, 225),
    cursor: rgb(125, 130, 172),
    accent: rgb(125, 130, 172),
    accent_fg: rgb(255, 255, 255),
    muted: rgb(170, 165, 180),
    success: rgb(100, 150, 110),
    warning: rgb(180, 140, 80),
    error: rgb(180, 80, 90),
    panel_bg: rgb(254, 249, 244),
    panel_border: rgb(172, 176, 214),
    panel_sel_bg: rgb(228, 223, 225),
    panel_sel_fg: rgb(80, 85, 120),
    mode_git: rgb(100, 150, 110),
    mode_term: rgb(100, 150, 110),
    mode_preview: rgb(140, 145, 185),
    mode_settings: rgb(140, 120, 170),
    mode_search: rgb(180, 140, 80),
    mode_find: rgb(125, 130, 172),
    git_add_bg: rgb(235, 245, 235),
    git_del_bg: rgb(250, 235, 235),
    git_hunk: rgb(125, 130, 172),
};

// ── MONO ──
pub static MONO: Theme = Theme {
    name: "mono",
    bg: rgb(255, 255, 255),
    fg: rgb(30, 30, 30),
    keyword: rgb(0, 0, 0),
    string: rgb(80, 80, 80),
    comment: rgb(170, 170, 170),
    number: rgb(50, 50, 50),
    type_name: rgb(60, 60, 60),
    function: rgb(20, 20, 20),
    macro_name: rgb(40, 40, 40),
    namespace: rgb(50, 50, 50),
    parameter: rgb(70, 70, 70),
    property: rgb(30, 30, 30),
    constant: rgb(40, 40, 40),
    operator: rgb(0, 0, 0),
    punctuation: rgb(150, 150, 150),
    line_no: rgb(190, 190, 190),
    current_line: rgba(30, 30, 30, 16),
    invisibles: rgb(219, 219, 219),
    editor_bg: rgb(255, 255, 255),
    model_bg: rgb(255, 255, 255),
    status_bg: rgb(245, 245, 245),
    status_fg: rgb(30, 30, 30),
    border: rgb(220, 220, 220),
    selection_bg: rgb(230, 230, 230),
    search_bg: rgb(255, 255, 200),
    mode_xlc: rgb(100, 100, 100),
    completion_bg: rgb(250, 250, 250),
    completion_selected: rgb(60, 60, 60),
    completion_border: rgb(200, 200, 200),
    explorer_bg: rgb(248, 248, 248),
    explorer_fg: rgb(30, 30, 30),
    explorer_dir: rgb(50, 50, 50),
    explorer_selected: rgb(220, 220, 220),
    terminal_bg: rgb(250, 250, 250),
    terminal_fg: rgb(30, 30, 30),
    terminal_prompt: rgb(80, 80, 80),
    xlc_bg: rgb(248, 248, 248),
    xlc_fg: rgb(50, 50, 50),
    xlc_prompt: rgb(80, 80, 80),
    xlc_border: rgb(200, 200, 200),
    cursor: rgb(0, 0, 0),
    accent: rgb(40, 40, 40),
    accent_fg: rgb(255, 255, 255),
    muted: rgb(140, 140, 140),
    success: rgb(40, 120, 40),
    warning: rgb(140, 110, 20),
    error: rgb(160, 40, 40),
    panel_bg: rgb(250, 250, 250),
    panel_border: rgb(180, 180, 180),
    panel_sel_bg: rgb(230, 230, 230),
    panel_sel_fg: rgb(0, 0, 0),
    mode_git: rgb(40, 120, 40),
    mode_term: rgb(40, 120, 40),
    mode_preview: rgb(60, 60, 60),
    mode_settings: rgb(80, 80, 80),
    mode_search: rgb(140, 110, 20),
    mode_find: rgb(60, 60, 60),
    git_add_bg: rgb(235, 250, 235),
    git_del_bg: rgb(255, 235, 235),
    git_hunk: rgb(80, 80, 80),
};

// ── MONO_DARK ──
// Light polish: muted was #646464 on #1e1e1e — too dim for status/meta chrome.
// Bumped toward #a0a0a0 so secondary labels stay readable without washing out.
pub static MONO_DARK: Theme = Theme {
    name: "mono_dark",
    bg: rgb(20, 20, 20),
    fg: rgb(230, 230, 230),
    keyword: rgb(255, 255, 255),
    string: rgb(190, 190, 190),
    comment: rgb(100, 100, 100),
    number: rgb(210, 210, 210),
    type_name: rgb(200, 200, 200),
    function: rgb(240, 240, 240),
    macro_name: rgb(220, 220, 220),
    namespace: rgb(210, 210, 210),
    parameter: rgb(200, 200, 200),
    property: rgb(230, 230, 230),
    constant: rgb(220, 220, 220),
    operator: rgb(255, 255, 255),
    punctuation: rgb(120, 120, 120),
    line_no: rgb(70, 70, 70),
    current_line: rgba(230, 230, 230, 16),
    invisibles: rgb(48, 48, 48),
    editor_bg: rgb(20, 20, 20),
    model_bg: rgb(255, 255, 255),
    status_bg: rgb(28, 28, 28),
    status_fg: rgb(230, 230, 230),
    border: rgb(50, 50, 50),
    selection_bg: rgb(60, 60, 60),
    search_bg: rgb(50, 50, 30),
    mode_xlc: rgb(170, 170, 170),
    completion_bg: rgb(30, 30, 30),
    completion_selected: rgb(200, 200, 200),
    completion_border: rgb(80, 80, 80),
    explorer_bg: rgb(16, 16, 16),
    explorer_fg: rgb(230, 230, 230),
    explorer_dir: rgb(210, 210, 210),
    explorer_selected: rgb(60, 60, 60),
    terminal_bg: rgb(24, 24, 24),
    terminal_fg: rgb(230, 230, 230),
    terminal_prompt: rgb(190, 190, 190),
    xlc_bg: rgb(26, 26, 26),
    xlc_fg: rgb(210, 210, 210),
    xlc_prompt: rgb(190, 190, 190),
    xlc_border: rgb(70, 70, 70),
    cursor: rgb(255, 255, 255),
    accent: rgb(200, 200, 200),
    accent_fg: rgb(0, 0, 0),
    muted: rgb(160, 160, 160),
    success: rgb(140, 200, 140),
    warning: rgb(200, 180, 100),
    error: rgb(220, 120, 120),
    panel_bg: rgb(30, 30, 30),
    panel_border: rgb(80, 80, 80),
    panel_sel_bg: rgb(55, 55, 55),
    panel_sel_fg: rgb(245, 245, 245),
    mode_git: rgb(140, 200, 140),
    mode_term: rgb(140, 200, 140),
    mode_preview: rgb(180, 180, 180),
    mode_settings: rgb(170, 170, 170),
    mode_search: rgb(200, 180, 100),
    mode_find: rgb(180, 180, 180),
    git_add_bg: rgb(25, 35, 25),
    git_del_bg: rgb(40, 22, 22),
    git_hunk: rgb(180, 180, 180),
};

/// Resolve a configured theme name against the current system appearance.
///
/// `"system"` (the default) follows macOS: a native app has no business
/// staying light while the rest of the desktop is dark. Any other name pins
/// that theme regardless of appearance.
pub fn resolve(name: &str, system_is_dark: bool) -> &'static Theme {
    if name.is_empty() || name == "system" {
        return if system_is_dark { &DARK } else { &LIGHT };
    }
    find(name).unwrap_or(if system_is_dark { &DARK } else { &LIGHT })
}

/// One addressable colour of a theme.
///
/// These are exactly the twenty the face already draws in its Themes preview,
/// so "what you can see" and "what you can change" are the same list. The
/// enum's ORDER is ABI — the face addresses a token by index. Append only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeToken {
    Fg,
    Comment,
    StringLit,
    Number,
    Keyword,
    TypeName,
    Function,
    MacroName,
    Namespace,
    Parameter,
    Property,
    Constant,
    Operator,
    Punctuation,
    LineNo,
    EditorBg,
    SelectionBg,
    Cursor,
    StatusBg,
    Accent,
    // Appended. The order of this enum is ABI — the face addresses a token
    // by index — so a new colour goes on the END even when it belongs
    // elsewhere in a list. Display order is the face's business.
    CurrentLine,
    Invisibles,
    ModelBg,
}

impl ThemeToken {
    pub const ALL: &'static [ThemeToken] = &[
        Self::Fg,
        Self::Comment,
        Self::StringLit,
        Self::Number,
        Self::Keyword,
        Self::TypeName,
        Self::Function,
        Self::MacroName,
        Self::Namespace,
        Self::Parameter,
        Self::Property,
        Self::Constant,
        Self::Operator,
        Self::Punctuation,
        Self::LineNo,
        Self::EditorBg,
        Self::SelectionBg,
        Self::Cursor,
        Self::StatusBg,
        Self::Accent,
        Self::CurrentLine,
        Self::Invisibles,
        Self::ModelBg,
    ];

    /// Key written into `~/.suisei.toml`. Stable — renaming one orphans a
    /// user's saved colour.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Fg => "fg",
            Self::Comment => "comment",
            Self::StringLit => "string",
            Self::Number => "number",
            Self::Keyword => "keyword",
            Self::TypeName => "type_name",
            Self::Function => "function",
            Self::MacroName => "macro_name",
            Self::Namespace => "namespace",
            Self::Parameter => "parameter",
            Self::Property => "property",
            Self::Constant => "constant",
            Self::Operator => "operator",
            Self::Punctuation => "punctuation",
            Self::LineNo => "line_no",
            Self::EditorBg => "editor_bg",
            Self::ModelBg => "model_bg",
            Self::SelectionBg => "selection_bg",
            Self::Cursor => "cursor",
            Self::StatusBg => "status_bg",
            Self::Accent => "accent",
            Self::CurrentLine => "current_line",
            Self::Invisibles => "invisibles",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Fg => "Plain Text",
            Self::Comment => "Comments",
            Self::StringLit => "Strings",
            Self::Number => "Numbers",
            Self::Keyword => "Keywords",
            Self::TypeName => "Type Names",
            Self::Function => "Function Names",
            Self::MacroName => "Macros",
            Self::Namespace => "Namespaces",
            Self::Parameter => "Parameters",
            Self::Property => "Properties",
            Self::Constant => "Constants",
            Self::Operator => "Operators",
            Self::Punctuation => "Punctuation",
            Self::LineNo => "Line Numbers",
            Self::EditorBg => "Editor Background",
            Self::SelectionBg => "Selection",
            Self::Cursor => "Cursor",
            Self::StatusBg => "Status Bar",
            Self::Accent => "Accent",
            Self::CurrentLine => "Current Line",
            Self::Invisibles => "Invisibles",
            Self::ModelBg => "3D Stage",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|t| t.key() == key)
    }

    /// Ink painted onto the editor background. These are the ones whose
    /// contrast against `EditorBg` decides whether code is readable, so they
    /// are the ones worth warning about.
    pub const fn is_editor_ink(self) -> bool {
        matches!(
            self,
            Self::Fg
                | Self::Comment
                | Self::StringLit
                | Self::Number
                | Self::Keyword
                | Self::TypeName
                | Self::Function
                | Self::MacroName
                | Self::Namespace
                | Self::Parameter
                | Self::Property
                | Self::Constant
                | Self::Operator
                | Self::Punctuation
                | Self::LineNo
                | Self::Invisibles
        )
    }

    pub fn get(self, theme: &Theme) -> Rgba {
        match self {
            Self::Fg => theme.fg,
            Self::Comment => theme.comment,
            Self::StringLit => theme.string,
            Self::Number => theme.number,
            Self::Keyword => theme.keyword,
            Self::TypeName => theme.type_name,
            Self::Function => theme.function,
            Self::MacroName => theme.macro_name,
            Self::Namespace => theme.namespace,
            Self::Parameter => theme.parameter,
            Self::Property => theme.property,
            Self::Constant => theme.constant,
            Self::Operator => theme.operator,
            Self::Punctuation => theme.punctuation,
            Self::LineNo => theme.line_no,
            Self::EditorBg => theme.editor_bg,
            Self::ModelBg => theme.model_bg,
            Self::SelectionBg => theme.selection_bg,
            Self::Cursor => theme.cursor,
            Self::StatusBg => theme.status_bg,
            Self::Accent => theme.accent,
            Self::CurrentLine => theme.current_line,
            Self::Invisibles => theme.invisibles,
        }
    }

    fn set(self, theme: &mut Theme, color: Rgba) {
        // Alpha is kept from the authored colour: several surfaces composite
        // over what is behind them (that is why `Rgba` has alpha at all), and
        // a colour well hands back an opaque value. Taking its alpha would
        // turn a translucent selection into a solid slab.
        let color = Rgba {
            a: self.get(theme).a,
            ..color
        };
        match self {
            Self::Fg => theme.fg = color,
            Self::Comment => theme.comment = color,
            Self::StringLit => theme.string = color,
            Self::Number => theme.number = color,
            Self::Keyword => theme.keyword = color,
            Self::TypeName => theme.type_name = color,
            Self::Function => theme.function = color,
            Self::MacroName => theme.macro_name = color,
            Self::Namespace => theme.namespace = color,
            Self::Parameter => theme.parameter = color,
            Self::Property => theme.property = color,
            Self::Constant => theme.constant = color,
            Self::Operator => theme.operator = color,
            Self::Punctuation => theme.punctuation = color,
            Self::LineNo => theme.line_no = color,
            Self::EditorBg => theme.editor_bg = color,
            Self::ModelBg => theme.model_bg = color,
            Self::SelectionBg => theme.selection_bg = color,
            Self::Cursor => theme.cursor = color,
            Self::StatusBg => theme.status_bg = color,
            Self::Accent => theme.accent = color,
            Self::CurrentLine => theme.current_line = color,
            Self::Invisibles => theme.invisibles = color,
        }
    }
}

/// Lay the user's per-token edits over an already-tinted palette.
///
/// Applied LAST, and each override sets exactly one field. That is the
/// difference between this and [`with_highlight`]: the highlight preference
/// means "make the accent this and re-derive everything downstream of it"
/// (selection, search, panel selection, hunk markers, the text drawn on
/// accent), whereas an override of [`ThemeToken::Accent`] means "this exact
/// colour, leave the rest alone". Both are useful and they are not the same
/// request, so both exist.
///
/// Unknown keys and malformed values are skipped rather than rejected — a
/// hand-edited config with one bad line still loads the other nineteen.
pub fn with_overrides(
    base: &Theme,
    overrides: Option<&std::collections::BTreeMap<String, String>>,
) -> Theme {
    let Some(overrides) = overrides else {
        return *base;
    };
    let mut theme = *base;
    for (key, value) in overrides {
        let (Some(token), Some(color)) = (ThemeToken::from_key(key), parse_hex(value)) else {
            continue;
        };
        token.set(&mut theme, color);
    }
    theme
}

/// The palette the editor actually paints with.
///
/// One function, because there are four places that needed the answer and each
/// was spelling out `with_highlight(resolve(…), …)` by hand — a third step
/// would have had to be added in four places and would have been forgotten in
/// at least one.
///
/// Overrides are keyed by the RESOLVED theme's name, not by the requested one.
/// With `theme = "system"` that means edits land under `light` or `dark` — the
/// palette you were actually looking at when you made them.
///
/// `name` is passed rather than read from `cfg` because one caller
/// (`apply_system_appearance`) has an in-session preference that may not have
/// reached the file yet, and folding that into `cfg.theme` would make a light/
/// dark switch silently revert an unsaved theme choice.
pub fn effective(name: &str, cfg: &crate::config::Config, system_is_dark: bool) -> Theme {
    let (base_name, key) = override_target(name, cfg, system_is_dark);
    let base = resolve(&base_name, system_is_dark);
    // A named palette lands whole, accent included.
    //
    // `highlight_color` is a tint for the two palettes that are a CANVAS —
    // Light and Dark are deliberately neutral, and the accent is the one
    // colour in them the user is expected to choose. Catppuccin is not a
    // canvas: its mauve is part of the palette, chosen against its own
    // background by whoever authored it, and letting a leftover highlight
    // preference repaint it means picking a theme gives you most of a theme.
    //
    // Per-token overrides still apply after this. Those are an explicit edit
    // to THIS palette; a highlight preference is a setting that happens to
    // still be lying around from a different one.
    let tinted = if takes_highlight_tint(&base_name) {
        with_highlight(base, &cfg.highlight_color)
    } else {
        *base
    };
    with_overrides(&tinted, cfg.theme_overrides.get(&key))
}

/// Whether a palette is a canvas to be tinted, or a finished thing.
pub fn takes_highlight_tint(resolved_name: &str) -> bool {
    matches!(resolved_name, "light" | "dark")
}

/// Which palette a theme is built on, and which override table belongs to it.
///
/// For a built-in, both are the resolved palette's name. For a user-made theme,
/// the base is whatever it was saved from and the table is the theme's OWN
/// name — that separation is what lets you edit "Midnight" without touching the
/// Catppuccin it started from.
///
/// Public because the settings panel and the face both need to name the table
/// they are writing into, and each deriving it independently is how the two
/// would drift.
pub fn override_target(
    name: &str,
    cfg: &crate::config::Config,
    system_is_dark: bool,
) -> (String, String) {
    match cfg.custom_themes.get(name) {
        Some(base) => (base.clone(), name.to_string()),
        None => {
            let resolved = resolve(name, system_is_dark).name.to_string();
            (resolved.clone(), resolved)
        }
    }
}

/// WCAG relative-contrast ratio, 1.0 (identical) to 21.0 (black on white).
///
/// This is the whole reason per-token editing is safe to offer. The palette
/// layer used to refuse the edits outright, on the grounds that letting each
/// colour drift independently is how unreadable themes get made. That reason
/// was sound and the conclusion was too strong: the fix for "you can make this
/// unreadable" is to say so, not to take the control away. 4.5 is WCAG AA for
/// body text; 3.0 is the large-text floor and a reasonable warning line for
/// syntax ink.
pub fn contrast_ratio(a: Rgba, b: Rgba) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Parse `#RRGGBB` (or bare `RRGGBB`). `default`/empty means "no override".
pub fn parse_hex(value: &str) -> Option<Rgba> {
    parse_hex_rgb(value)
}

/// Apply the one *derived* palette override: the interaction/highlight hue.
///
/// The chosen hue is used directly for controls and softly mixed into
/// selection/search surfaces; `default` leaves the authored palette intact.
/// For setting a single colour and nothing else, see [`with_overrides`].
pub fn with_highlight(base: &Theme, preference: &str) -> Theme {
    let Some(highlight) = parse_hex_rgb(preference) else {
        return *base;
    };
    let mut theme = *base;
    theme.accent = highlight;
    theme.accent_fg = if relative_luminance(highlight) > 0.56 {
        rgb(18, 20, 24)
    } else {
        rgb(255, 255, 255)
    };
    theme.selection_bg = mix(
        highlight,
        base.editor_bg,
        if base.name == "light" { 0.76 } else { 0.62 },
    );
    theme.search_bg = mix(
        highlight,
        base.editor_bg,
        if base.name == "light" { 0.68 } else { 0.54 },
    );
    theme.panel_sel_bg = mix(
        highlight,
        base.panel_bg,
        if base.name == "light" { 0.82 } else { 0.68 },
    );
    theme.mode_search = highlight;
    theme.mode_find = highlight;
    theme.git_hunk = highlight;
    theme
}

fn parse_hex_rgb(value: &str) -> Option<Rgba> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("default") {
        return None;
    }
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some(rgb(
        ((n >> 16) & 0xff) as u8,
        ((n >> 8) & 0xff) as u8,
        (n & 0xff) as u8,
    ))
}

fn mix(foreground: Rgba, background: Rgba, background_amount: f32) -> Rgba {
    let amount = background_amount.clamp(0.0, 1.0);
    let channel =
        |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * amount).round() as u8;
    rgb(
        channel(foreground.r, background.r),
        channel(foreground.g, background.g),
        channel(foreground.b, background.b),
    )
}

fn relative_luminance(color: Rgba) -> f32 {
    let linear = |v: u8| {
        let value = f32::from(v) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    linear(color.r) * 0.2126 + linear(color.g) * 0.7152 + linear(color.b) * 0.0722
}

#[cfg(test)]
mod resolve_tests {
    use super::*;

    #[test]
    fn system_follows_the_appearance() {
        assert_eq!(resolve("system", true).name, "dark");
        assert_eq!(resolve("system", false).name, "light");
        assert_eq!(resolve("", true).name, "dark");
    }

    #[test]
    fn an_explicit_theme_pins_regardless_of_appearance() {
        assert_eq!(resolve("ocean", true).name, "ocean");
        assert_eq!(resolve("ocean", false).name, "ocean");
    }

    #[test]
    fn an_unknown_name_falls_back_to_the_appearance() {
        assert_eq!(resolve("nope", true).name, "dark");
    }

    #[test]
    fn highlight_override_changes_semantic_highlights_only() {
        let customized = with_highlight(&DARK, "#FF2D55");
        assert_eq!(customized.accent, rgb(255, 45, 85));
        assert_eq!(customized.keyword, DARK.keyword);
        assert_eq!(customized.editor_bg, DARK.editor_bg);
        assert_ne!(customized.selection_bg, DARK.selection_bg);
    }

    #[test]
    fn default_or_invalid_highlight_keeps_authored_palette() {
        assert_eq!(with_highlight(&LIGHT, "default").accent, LIGHT.accent);
        assert_eq!(with_highlight(&LIGHT, "not-a-colour").accent, LIGHT.accent);
    }
}
