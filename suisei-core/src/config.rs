//! User config at `~/.xei.toml` (simple line-oriented, no extra deps).

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub theme: String,
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
    /// Kitty-graphics layer (inline preview images, pet, media previews).
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
    /// Desktop pet GIF overlay (requires gpu_acc + Kitty graphics).
    pub pet_enabled: bool,
    pub pet_path: String,
    pub pet_x: u16,
    pub pet_y: u16,
    pub pet_width_cells: u16,
    /// Playback speed percent (25..=400). 100 = native GIF timing.
    pub pet_speed: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: "ocean".into(),
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
            pet_enabled: false,
            pet_path: String::new(),
            pet_x: 2,
            pet_y: 2,
            pet_width_cells: 12,
            pet_speed: 100,
        }
    }
}

/// Languages shown in Settings → LSP (order preserved).
pub fn lsp_lang_catalog() -> &'static [(&'static str, &'static str, &'static str)] {
    // (settings key, display label, default command)
    &[
        ("rust", "Rust", "rust-analyzer"),
        ("python", "Python", "pyright-langserver --stdio"),
        ("typescript", "TypeScript", "typescript-language-server --stdio"),
        ("javascript", "JavaScript", "typescript-language-server --stdio"),
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
    ]
}

fn config_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".xei.toml")
}

pub fn load() -> Config {
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
            "pet_enabled" | "pet" => {
                cfg.pet_enabled = matches!(v, "true" | "1" | "yes" | "on");
            }
            "pet_path" => {
                cfg.pet_path = v.to_string();
            }
            "pet_x" => {
                if let Ok(n) = v.parse::<u16>() {
                    // Soft cap only; runtime clamps to terminal size.
                    cfg.pet_x = n.min(10_000);
                }
            }
            "pet_y" => {
                if let Ok(n) = v.parse::<u16>() {
                    cfg.pet_y = n.min(10_000);
                }
            }
            "pet_width_cells" | "pet_width" => {
                if let Ok(n) = v.parse::<u16>() {
                    cfg.pet_width_cells = n.clamp(4, 80);
                }
            }
            "pet_speed" => {
                if let Ok(n) = v.parse::<u16>() {
                    cfg.pet_speed = n.clamp(25, 400);
                }
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
        "# xei config\ntheme = \"{}\"\ntab_width = {}\nclipboard_sync = {}\nrelative_number = {}\nwrap_lines = {}\nupdate_check = {}\nundo_caching = {}\ngpu_graphics = {}\ngpu_hyperlinks = {}\ngpu_acc = {}\nkey_hints = {}\nlsp_enabled = {}\npet_enabled = {}\npet_path = \"{}\"\npet_x = {}\npet_y = {}\npet_width_cells = {}\npet_speed = {}\n",
        cfg.theme,
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
        if cfg.pet_enabled { "true" } else { "false" },
        cfg.pet_path.replace('"', ""),
        cfg.pet_x,
        cfg.pet_y,
        cfg.pet_width_cells,
        cfg.pet_speed.clamp(25, 400),
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
