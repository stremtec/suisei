# Navigator header actions

Design + plan, 2026-07-29. Approved in conversation (placement: user; treatment:
proposed here, greenlit with "진행").

## Problem

The navigator card wastes its top seam: ~23 pt of dead space between the
traffic-light band and the mode strip, and the tree's only creation affordance
is a context menu. The filter footer's bare `+` (New Untitled Tab — a TUI
vestige; Xcode uses that slot for file creation) duplicates the tab strip's
`+` menu.

## Design

**Action row** — right end of the navigator's TITLE row (the existing
`dockedNavigator` header that carries the mode name — "Project" — and used
to carry a lone circular refresh). Section-header grammar: label left,
actions right, one row (user-directed: "알약 바로 아래, project라고 써진 곳
오른쪽"). The earlier attempt put the icons in the tree's root row, which
stacked two header-like rows and tangled the layout.

```
│ ╭──────────────────────────╮ │
│ │ (pill) proj scm find brk │ │  mode strip ─┐ lifted 12 pt
│ │  Project    📄+ 📁+ ⟳ ◁  │ │  title row + actions
│ │  ▾ project               │ │  tree root
│ │    src                   │ │
│ │  ⌕ Filter ──────────     │ │  filter (bare + retired)
```

The title-row buttons post notifications (`.suiseiNavNewFile` /
`NewFolder` / `CollapseAll`) that `ProjectTreeView` executes — the state
they need (expansion set, inline rename on the fresh entry) lives there.
Refresh is the header's own action (cache invalidate + `ensureProjectTree`),
unchanged from the old circular button, restyled. The circular button's
ad-hoc styling is gone; the row now uses the topBar's `ToolbarPlainIcon`.

- Widget: `ToolbarPlainIcon` verbatim — the topBar's own widget, zero new
  visual vocabulary. `iconSize: 12` (one step quieter than the topBar's 13 —
  in-card secondary chrome, same size as the filter's old `+`).
- Glyphs: `doc.badge.plus` (New File in Folder), `folder.badge.plus` (New
  Folder), `arrow.clockwise` (Refresh), `chevron.left.square` (Collapse All).
- Ink/motion: the widget's own — dim 0.85 → hover dim + capsule 0.07 → press
  0.90. No container, no separator line (card grammar: spacing, not rules).
- Alignment: leading 8 pt (= filter footer's leading), `HStack(spacing: 0)` —
  the 26 pt hit boxes carry the rhythm.

**Actions**
- New File / New Folder → `newEntry(in: targetFolder, folder:)` — the context
  menu's path, including its inline rename on the fresh entry.
- Target cascade: `selectedPath` (dir → itself, file → its parent) → active
  document's folder when it lives under `rootPath` (`engine.chrome.filename`)
  → `rootPath`. The tree already tracks `selectedPath`; no new state.
- Refresh → `onRefresh()`.
- Collapse All → `withAnimation(.smooth(0.26)) { expanded = [rootPath] }`
  (root stays open, everything else folds — the tree's own animation).

**Footer `+` retired** — New Untitled Tab lives in the tab strip's `+` menu;
two pluses stacked (bare `+` under `doc.badge.plus`) would read as one
feature. The footer becomes the pure filter capsule.

**Strip lift** — the sidebar's top spacer 48 → 36 pt: the mode strip rises
12 pt (lights↔strip 23 → ~5 pt), the action row lands where the strip used to
sit, and the tree barely moves. Tab-strip clearance verified: the topBar's
centering reserves `navW + 16` on the left, so centered tabs never cross the
card regardless of the lift.

## Scope

Face-only (`ProjectTreeView.swift`, `ContentView.swift`). No core/engine/ABI
changes. No unit-test infrastructure for SwiftUI faces — verification is
build + visual (icons render/align, each action works, lift looks right,
narrow-window tab clearance).

## Out of scope (YAGNI)

Tree multi-selection, drag-target-aware creation, hover-revealed actions
(always-visible matches Xcode), per-row action menus.
