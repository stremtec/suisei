# Suisei Settings Plan

## Product direction

Suisei Settings should combine two native macOS patterns without imitating
either application literally:

- Xcode supplies the information architecture: a compact monochrome sidebar,
  a page title in the toolbar, and dense single-column groups of real controls.
- System Settings supplies the window anatomy: a full-height sidebar, search
  below the traffic lights, and a pinned material that blurs rows scrolling
  underneath it.

The UI must not expose a control until there is a durable setting or action
behind it. Future destinations may explain their status, but must not contain
fake sign-in, update, install, or permission flows.

## What was inspected in Xcode 26.6

The running Xcode Settings window was inspected directly, including General,
Apple Accounts, Behaviors, Navigation, Themes, Editing, Source Control,
Components, and Locations.

The recurring patterns are:

- 188pt source-list sidebar with monochrome, outline-style symbols;
- a native back/forward `ControlGroup` followed by the current page title;
- section headings outside rounded groups;
- one control per row, with the control aligned to the trailing edge;
- secondary descriptions only when a setting has a non-obvious consequence;
- segmented controls only for a small, exclusive set of modes;
- no centered hero card and no page made from links to the other sidebar pages;
- account onboarding is one compact explanatory card plus one clear action;
- component and location pages present state and paths as lists, not dashboards.

## Proposed sidebar

The final list should be driven by implemented capabilities, in this order:

1. **General** — appearance mode, core editor/file defaults, rendering, language
   intelligence.
2. **Accounts** — Google account, sync scope, and connection state. Keep the
   current status-only page until OAuth and credential storage exist.
3. **Behaviors** — event-to-action rules for build, run, test, search, files,
   and custom commands. Requires a behavior model in Core first.
4. **Navigation** — tab/pane opening, definition jumps, modifier-click actions,
   and navigator behavior. Requires stable navigation preferences in Core.
5. **Themes** — application appearance, syntax theme, font, line spacing,
   cursor, selection, and semantic-token colours. This grows from Appearance.
6. **Editing** — encoding, line endings, whitespace, indentation, wrapping,
   relative numbers, undo cache, clipboard sync, and key hints.
7. **Shortcuts** — searchable command table, editable bindings, conflict
   detection, reset, import, and export.
8. **Source Control** — enablement, refresh/fetch behavior, default branch,
   comparison orientation, and repository integration. Opening a workbench is
   an editor command, not a preference.
9. **Extensions** — installed extensions, enabled state, version, updates, and
   removal. Do not advertise a marketplace until the host can install safely.
10. **Components** — language servers, runtimes, grammars, and toolchains with
    installed/missing status and explicit install actions.
11. **Locations** — config, cache, logs, extensions, language servers, and
    project indexes, with reveal/change actions where safe.
12. **Software Update** — app and engine versions, release channel, last check,
    automatic checks, and update result. Requires a signed update mechanism.

`About` is not a settings destination. It uses a dedicated native application
panel from the Suisei menu, populated by the app icon, bundle version,
copyright metadata, and the shipped license. Engine, theme, and
configuration-path diagnostics belong in Locations or a future diagnostics
surface, not in the About panel.

## Page composition rules

- Start with the first real section; never add a decorative page hero.
- Use one vertical scan path. Avoid grids of unrelated settings and nested
  cards used only as navigation.
- Keep rows at native control height. Add a second line only for behavior,
  safety, storage, or performance consequences.
- Use semantic colours and native controls. Sidebar icons are monochrome; colour
  tiles are reserved for content that is itself an icon asset or status.
- Search is pinned in a bar material. Its initial top margin keeps the first row
  visible, but that margin scrolls away so later rows pass under the blur.
- Use `NavigationControlGroupStyle` for history; do not draw another pill around
  toolbar buttons.
- Prefer immediate persistence like Xcode. Until Core supports debounced,
  failure-aware writes, show the existing Save bar only while changes are dirty.

## Data model work

The current Core contract exposes four implementation pages and a flat row list.
Swift temporarily routes several visual destinations to Core page 1. Replace
this with a schema containing stable setting keys, group identifiers, value
types, constraints, restart requirements, search terms, and persistence status.

Suggested sequence:

1. Add stable setting keys and page/group metadata without changing behavior.
2. Move General, Themes, Editing, and Components onto that schema.
3. Add debounced persistence with an error result; remove the global Save bar
   only after failed writes can be surfaced per setting.
4. Add Behaviors, Navigation, Source Control preferences, and Locations as
   their Core models become real.
5. Add Accounts and Software Update last, with Keychain-backed OAuth and a
   signed updater rather than placeholder controls.

### Implemented native presentation contract

Core now exports more than a stable row identity. Every editable row carries
its native destination, group title, control type, explanatory detail, option
list, selected option, and whether it belongs behind Advanced. The face sends
an explicit option index back to Core instead of simulating a picker by cycling
the TUI action.

The first layout pass uses that contract as follows:

- General contains appearance and Editor Defaults. Line Numbers and Line
  Wrapping are menus; Tab Width is segmented. It contains no switches.
- Editor contains three ordinary binary preferences. Terminal-only GPU options
  are collapsed under Advanced.
- Language Servers contains one enable switch and per-language server modes.
- Source Control contains actions rather than pretending editor commands are
  persisted preferences.

This is an ABI contract. `SettingRow` identity, native page codes, and control
codes are append-only, and explicit picker selection is covered by Core tests.

## Acceptance checks

- At rest, the first sidebar row is fully visible below search.
- While the sidebar scrolls, rows move beneath a visibly blurred search bar;
  the search region must be an overlay, not a safe-area inset that clips rows.
- Back/forward matches the native Xcode toolbar control in size and disabled
  state and preserves forward history until a new branch is selected.
- Every visible control has a working Core action or is clearly read-only.
- Light, dark, active, inactive, reduced-transparency, and increased-contrast
  appearances remain legible without hard-coded dark colours.
