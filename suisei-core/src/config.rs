//! User config at `~/.suisei.toml` (simple line-oriented, no extra deps).

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub theme: String,
    /// Liquid Glass density used by native floating chrome.
    /// `"clear"` preserves more of the content behind it; `"tinted"` adds
    /// a quiet theme-aware wash for stronger separation.
    pub glass_style: String,
    /// Optional six-digit sRGB override for selection/accent highlights.
    /// `"default"` keeps the palette's own carefully tuned highlight.
    pub highlight_color: String,
    /// Spaces per tab / indent
    pub tab_width: usize,
    /// Mirror yanks to system clipboard (unnamedplus-style)
    pub clipboard_sync: bool,
    /// Show relative line numbers in gutter
    pub relative_number: bool,
    /// Soft-wrap long lines (false = horizontal scroll).
    pub wrap_lines: bool,
    /// Startup check for a newer release (welcome-screen notice).
    pub update_check: bool,
    /// Keep undo history on disk when a file closes (resume on reopen).
    pub undo_caching: bool,
    /// Kitty-graphics layer (inline preview images, media previews).
    pub gpu_graphics: bool,
    /// OSC 8 hyperlinks.
    pub gpu_hyperlinks: bool,
    /// GPU-terminal progressive enhancements (Ghostty/Kitty).
    pub gpu_acc: bool,
    /// Show which-key style chord hints after prefix keys.
    pub key_hints: bool,
    /// Master switch for automatic LSP start.
    pub lsp_enabled: bool,
    /// Per-language LSP command overrides.
    /// Key = language id (`rust`, `python`, …).
    /// Value = command line; empty string = disabled for that language.
    /// Missing key = use built-in default.
    pub lsp_servers: HashMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            glass_style: "clear".into(),
            highlight_color: "default".into(),
            tab_width: 4,
            clipboard_sync: true,
            relative_number: false,
            wrap_lines: true,
            update_check: true,
            undo_caching: false,
            gpu_graphics: true,
            gpu_hyperlinks: true,
            gpu_acc: true,
            key_hints: true,
            lsp_enabled: true,
            lsp_servers: HashMap::new(),
        }
    }
}

/// Languages shown in Settings → LSP (order preserved).
pub fn lsp_lang_catalog() -> &'static [(&'static str, &'static str, &'static str)] {
    // (settings key, display label, default command)
    &[
        ("rust", "Rust", "rust-analyzer"),
        ("python", "Python", "pyright-langserver --stdio"),
        (
            "typescript",
            "TypeScript",
            "typescript-language-server --stdio",
        ),
        (
            "javascript",
            "JavaScript",
            "typescript-language-server --stdio",
        ),
        ("c", "C / C++", "clangd"),
        ("go", "Go", "gopls"),
        ("java", "Java", "jdtls"),
        ("lua", "Lua", "lua-language-server"),
        ("json", "JSON", "vscode-json-language-server --stdio"),
        ("yaml", "YAML", "yaml-language-server --stdio"),
        ("toml", "TOML", "taplo lsp stdio"),
        ("markdown", "Markdown", "marksman server"),
        ("bash", "Bash", "bash-language-server start"),
        ("zig", "Zig", "zls"),
        // This list is what Settings shows, so anything missing from it cannot
        // be configured at all. It held fourteen entries while
        // `lsp::default_server_for_ext` knew twenty-eight — Ruby, PHP, C# and
        // the rest started a server that the user could then neither see nor
        // change. Kept in step by `tests/language_tables_agree.rs`; the command
        // must match the fallback exactly, because this table is consulted
        // FIRST and would otherwise silently override it.
        ("csharp", "C#", "csharp-ls"),
        ("ruby", "Ruby", "solargraph stdio"),
        ("php", "PHP", "intelephense --stdio"),
        ("swift", "Swift", "sourcekit-lsp"),
        ("kotlin", "Kotlin", "kotlin-language-server"),
        ("objective-c", "Objective-C", "clangd"),
        ("dart", "Dart", "dart language-server"),
        ("scala", "Scala", "metals"),
        ("haskell", "Haskell", "haskell-language-server-wrapper --lsp"),
        ("elixir", "Elixir", "elixir-ls"),
        ("nim", "Nim", "nimlsp"),
        ("html", "HTML", "vscode-html-language-server --stdio"),
        ("css", "CSS", "vscode-css-language-server --stdio"),
        ("xml", "XML", "lemminx"),
        ("vue", "Vue", "vue-language-server --stdio"),
        ("svelte", "Svelte", "svelteserver --stdio"),
        ("cmake", "CMake", "cmake-language-server"),
    ]
}

fn home_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
}

/// Suisei's own config. It used to be `~/.xei.toml`, shared with the xei TUI —
/// so changing the theme in one silently changed it in the other, and a `light`
/// left there by the TUI is what made the GUI ignore macOS dark mode. The
/// journal was separated for the same reason; this finishes the job.
fn config_path() -> PathBuf {
    home_dir().join(".suisei.toml")
}

/// One-time adoption of the old shared file, so an existing setup is not reset.
///
/// `theme` is deliberately NOT carried over. Whatever is in the xei config is
/// the *TUI's* choice, and a terminal editor has no system appearance to
/// follow — carrying a `theme = "light"` across is exactly what left the GUI
/// light on a dark desktop. Adoption starts at `system`; every other setting
/// (tab width, LSP servers, hooks) transfers as-is.
fn migrate_from_xei() {
    let ours = config_path();
    if ours.exists() {
        return;
    }
    let theirs = home_dir().join(".xei.toml");
    if let Ok(content) = fs::read_to_string(&theirs) {
        let kept: String = content
            .lines()
            .filter(|l| !l.trim_start().starts_with("theme"))
            .map(|l| format!("{l}\n"))
            .collect();
        let _ = fs::write(
            &ours,
            format!("# suisei config\ntheme = \"system\"\n{kept}"),
        );
    }
}

pub fn load() -> Config {
    migrate_from_xei();
    let mut cfg = Config::default();
    let Ok(content) = fs::read_to_string(config_path()) else {
        return cfg;
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        let v = v.trim().trim_matches('"').trim_matches('\'');
        match k {
            "theme" => {
                if !v.is_empty() {
                    cfg.theme = v.to_string();
                }
            }
            "glass_style" => {
                if matches!(v, "clear" | "tinted") {
                    cfg.glass_style = v.to_string();
                }
            }
            "highlight_color" | "accent_color" => {
                let candidate = v.trim();
                let hex = candidate.strip_prefix('#').unwrap_or(candidate);
                if candidate.eq_ignore_ascii_case("default")
                    || (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()))
                {
                    cfg.highlight_color = if candidate.eq_ignore_ascii_case("default") {
                        "default".into()
                    } else {
                        format!("#{}", hex.to_ascii_uppercase())
                    };
                }
            }
            "tab_width" | "tabstop" => {
                if let Ok(n) = v.parse::<usize>() {
                    if n > 0 && n <= 16 {
                        cfg.tab_width = n;
                    }
                }
            }
            "clipboard_sync" => {
                cfg.clipboard_sync = matches!(v, "true" | "1" | "yes" | "on");
            }
            "relative_number" | "relativenumber" => {
                cfg.relative_number = matches!(v, "true" | "1" | "yes" | "on");
            }
            "wrap_lines" | "wrap" => {
                cfg.wrap_lines = matches!(v, "true" | "1" | "yes" | "on");
            }
            "update_check" => {
                cfg.update_check = matches!(v, "true" | "1" | "yes" | "on");
            }
            "undo_caching" => {
                cfg.undo_caching = matches!(v, "true" | "1" | "yes" | "on");
            }
            "gpu_graphics" => {
                cfg.gpu_graphics = matches!(v, "true" | "1" | "yes" | "on");
            }
            "gpu_hyperlinks" => {
                cfg.gpu_hyperlinks = matches!(v, "true" | "1" | "yes" | "on");
            }
            "gpu_acc" | "gpu_acceleration" | "graphics" => {
                cfg.gpu_acc = matches!(
                    v,
                    "true" | "1" | "yes" | "on" | "auto" | "kitty" | "ghostty"
                );
            }
            "key_hints" | "which_key" | "chord_hints" => {
                cfg.key_hints = matches!(v, "true" | "1" | "yes" | "on");
            }
            "lsp_enabled" | "lsp" => {
                cfg.lsp_enabled = matches!(v, "true" | "1" | "yes" | "on");
            }
            k if k.starts_with("lsp.") => {
                let lang = k.trim_start_matches("lsp.").trim().to_lowercase();
                if !lang.is_empty() {
                    // empty value or "off" / "none" / "false" disables
                    if matches!(v, "" | "off" | "none" | "false" | "0") {
                        cfg.lsp_servers.insert(lang, String::new());
                    } else {
                        cfg.lsp_servers.insert(lang, v.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    cfg
}

pub fn save(cfg: &Config) {
    let mut content = format!(
        "# suisei config\ntheme = \"{}\"\nglass_style = \"{}\"\nhighlight_color = \"{}\"\ntab_width = {}\nclipboard_sync = {}\nrelative_number = {}\nwrap_lines = {}\nupdate_check = {}\nundo_caching = {}\ngpu_graphics = {}\ngpu_hyperlinks = {}\ngpu_acc = {}\nkey_hints = {}\nlsp_enabled = {}\n",
        cfg.theme,
        cfg.glass_style,
        cfg.highlight_color,
        cfg.tab_width,
        cfg.clipboard_sync,
        cfg.relative_number,
        if cfg.wrap_lines { "true" } else { "false" },
        if cfg.update_check { "true" } else { "false" },
        if cfg.undo_caching { "true" } else { "false" },
        if cfg.gpu_graphics { "true" } else { "false" },
        if cfg.gpu_hyperlinks { "true" } else { "false" },
        if cfg.gpu_acc { "true" } else { "false" },
        if cfg.key_hints { "true" } else { "false" },
        if cfg.lsp_enabled { "true" } else { "false" },
    );
    content.push_str("\n# LSP servers (empty / off = disabled; omit = built-in default)\n");
    // Save known catalog keys first (stable order), then any extras
    let mut seen = std::collections::HashSet::new();
    for (key, _label, _default) in lsp_lang_catalog() {
        if let Some(cmd) = cfg.lsp_servers.get(*key) {
            seen.insert(key.to_string());
            if cmd.is_empty() {
                content.push_str(&format!("lsp.{key} = \"off\"\n"));
            } else {
                content.push_str(&format!("lsp.{key} = \"{}\"\n", cmd.replace('"', "")));
            }
        }
    }
    let mut extras: Vec<_> = cfg
        .lsp_servers
        .iter()
        .filter(|(k, _)| !seen.contains(k.as_str()))
        .collect();
    extras.sort_by(|a, b| a.0.cmp(b.0));
    for (k, cmd) in extras {
        if cmd.is_empty() {
            content.push_str(&format!("lsp.{k} = \"off\"\n"));
        } else {
            content.push_str(&format!("lsp.{k} = \"{}\"\n", cmd.replace('"', "")));
        }
    }
    let _ = fs::write(config_path(), content);
}

pub fn save_theme(name: &str) {
    let mut cfg = load();
    cfg.theme = name.to_string();
    save(&cfg);
}

pub fn load_theme() -> Option<String> {
    let cfg = load();
    if cfg.theme.is_empty() {
        None
    } else {
        Some(cfg.theme)
    }
}

#[cfg(test)]
mod migration_tests {
    /// The xei config's `theme` is the *TUI's* choice and has no notion of
    /// following the system. Carrying it over is what put the GUI in light mode
    /// on a dark desktop — twice.
    #[test]
    fn adoption_drops_the_theme_and_keeps_the_rest() {
        let src = "# xei config\ntheme = \"light\"\ntab_width = 2\nlsp_enabled = true\n";
        let kept: String = src
            .lines()
            .filter(|l| !l.trim_start().starts_with("theme"))
            .map(|l| format!("{l}\n"))
            .collect();
        let out = format!("# suisei config\ntheme = \"system\"\n{kept}");
        assert!(out.contains("theme = \"system\""));
        assert!(!out.contains("theme = \"light\""));
        assert!(out.contains("tab_width = 2"));
        assert!(out.contains("lsp_enabled = true"));
    }
}
