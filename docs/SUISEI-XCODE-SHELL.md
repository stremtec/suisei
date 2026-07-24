# Suisei Shell v2 — Xcode-like layout

## Goals
- GUI face follows **Xcode window anatomy**, not TUI floating cards.
- Core / keys stay xei-compatible.
- Welcome window mirrors **Xcode 26 launch sheet**.

## Window anatomy

```
┌ Toolbar (glass) ──────────────────────────────────────────┐
│ 晴  · editor tabs · branch ·  [Nav] [Debug] [Insp] · ⚙     │
├──┬ Navigator ───┬──────── Editor ──────────┬─ Inspector ─┤
│█ │ Project tree │ breadcrumbs              │ (later)     │
│█ │ or SCM       │ source + gutter          │             │
│█ │ or Find      │ or Git docked view       │             │
├──┴──────────────┴──────────────────────────┴─────────────┤
│ Debug area: Terminal | XLC | DAP                           │
└────────────────────────────────────────────────────────────┘
```

### Navigator icon rail (left of nav content)
| Icon | Mode | Core mapping |
|------|------|----------------|
| folder | Project | Explorer |
| arrow.triangle.branch | SCM | Source Control |
| magnifyingglass | Find | Workspace search (later) |
| ant | Debug | DAP (later) |

### Debug area tabs
| Tab | Core |
|-----|------|
| Terminal | Ctrl+T |
| XLC | `:` |
| Console | DAP (later) |

### Welcome (no file / blank)
Xcode-style card:
- Create New Project → blank buffer + leave welcome
- Clone Git Repository → URL/path → open
- Open Existing → folder or file
- Recents list (UserDefaults)

## Implementation phases
1. Welcome window + recents (this PR)
2. Docked navigator rail + Project/SCM panels (no float)
3. Debug area bottom split for Terminal/XLC
4. Inspector placeholder
5. Git full client as editor-slot dock (already partial)

## Shell hygiene (Xcode 26 discipline)
- **Debug area** is closed by default. Opens only on Ctrl+T / `:` / rail toggle. Closing with ✕ hides it (no sticky `|| true`).
- **Project tree** is docked and **mode-independent**: entries always painted; opening a file returns Mode::Normal but keeps `explorer.entries`. Use `ensure_project_tree` (not Ctrl+F toggle) so the editor keeps Normal keys.
- **Chrome** is flat (tab bar / breadcrumbs / status / navigator) — Liquid Glass only for floating overlays (palette, settings, which-key).
- **Welcome exit on folder**: `open_path` on a directory opens preferred first file (or Untitled under root) so `welcome=false`. Recents folders work.
- **Jump bar**: path segments under tab bar; outline menu at end.
- **Inspector**: right Outline panel from Core structure scan (`#` / fn / struct…); click → `goto_line`.
