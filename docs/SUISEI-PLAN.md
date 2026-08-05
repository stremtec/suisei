# Suisei (彗星) — Design Plan

> **Status:** implementation active; code-verified 2026-07-23. Face = **Swift**
> (locked). Old Tauri scaffold deleted. Engine: `suisei-engine` (Rust cdylib) +
> `suisei-app` (SwiftUI/AppKit). The actual editing crate is **`suisei-core`**,
> a fork of `xei-core`; it is not automatically shared with the TUI.
>
> This document is the target architecture and contains historical phase notes.
> For implemented capability use `SUISEI-GAP.md`; for the next independence
> patches use `SUISEI-CURRENT-STATE.md`.

---

## 1. Product thesis

| Pillar | Meaning |
|--------|---------|
| **Same brain as xei** | Suisei carries the xei editing model in `suisei-core`; Swift never reimplements buffer/Vim operations. Fork parity is deliberate work, not automatic sharing. |
| **Same fingers as xei** | One `KeyEvent` → `dispatch` design. Since the Core is forked, parity must be enforced by shared fixtures/tests rather than assumed. |
| **Faster than VS Code** | Rust owns the hot path: buffer, layout metrics, syntax tokens, diff, git, process supervision. The UI shell is a **presentation face**, not the engine. |
| **Looks native, not Electron** | Liquid-glass / vibrancy chrome on macOS; dense IDE layout language borrowed from xei (tabs, panels, status, palette). Renderer tech is chosen to serve this — see §3.6. |
| **Extensions where they matter** | Headless VS Code extensions via shared host; full webviews only in Suisei (TUI degrades). |

**Non-goals (v1 desktop):**
- Windows/Linux pixel-perfect glass (macOS first; other OS later with simpler chrome).
- 100% VS Code webview/workbench API day one.
- Shipping suisei as the primary install before xei TUI stability regresses.

**UI source of truth:** Suisei looks and is organized **like xei’s TUI** (same regions, same mode surfaces, same muscle memory). The desktop shell upgrades **fidelity and things the terminal cannot do** — it does not invent a second IDE layout. Analysis of current TUI: §1.1–1.5 below (`xei/src/ui.rs` `draw`).

---

## 1.1 xei TUI — screen map (must clone in Suisei)

Primary shell (everything except exclusive full-screens):

```
┌──────────────────────────────────────────────────────────────────┐
│ TAB BAR          [files…]                          (hit regions) │  1 row
├──────────────────────────────────────────────────────────────────┤
│ BREADCRUMBS      path › segments                   (optional)    │  1 row
├────┬─────┬───────────────────────────────────┬───────────────────┤
│ACT │ EXPL│  MAIN (editor / git / term / …)   │ SIDE TERM (opt)   │
│BAR │ OR  │                                   │                   │
│EXT │     │  ± blame column (slide)           │                   │
│34c │     │  ± debug dock (bottom of main)    │                   │
│    │     │  ± XLC strip (bottom of main)     │                   │
├────┴─────┴───────────────────────────────────┴───────────────────┤
│ SEARCH BAR (only Mode::Search)                                   │  1 row
├──────────────────────────────────────────────────────────────────┤
│ STATUS  mode · branch · GPU · DAP · message · LSP · pos · %      │  1 row
└──────────────────────────────────────────────────────────────────┘
```

**Horizontal body (left → right):**
1. **Extensions panel** (optional, 34 cols) = activity bar (3) + list  
2. **Explorer** (optional, `explorer_width`, drag-resize)  
3. **Main** (`Min`) — editor / full surfaces  
4. **Side terminal** (optional, `terminal_width`, drag-resize)

**Main vertical (when open):**
- Editor (or git / full term / debug stack)  
- **XLC** command panel height (`xlc_height`)  
- **DAP** panel docks *inside* editor area (slide-up), not outer chrome

**Z-order (bottom → top in `draw`):**
1. Base layout (tabs → body → status)  
2. In-main surfaces (editor / git workbench / preview-in-pane / term / debug)  
3. Floating / modal: peek, workspace search, completions, which-key, palette, call hierarchy, rebase, PR review, editor ctx menu  
4. Side overlays: SCM slide, Settings  
5. Hover card  
6. Pet marker (fallback)

**Exclusive full-screen modes** (replace entire chrome):  
`Screensaver` · `Bench` · `PluginStore` · `Webview`

---

## 1.2 Surface inventory (1:1 → Suisei)

| Surface | Mode / flag | xei draw | Suisei parity |
|---------|-------------|----------|---------------|
| Tab bar | always | `draw_tabbar` | Same chips + dirty · mouse |
| Breadcrumbs | file open | `draw_breadcrumbs` | Same path segments; clickable free |
| Editor | Normal/Insert/Visual* | `draw_editor` + soft-wrap | **Core paint target** |
| Multi-split | split state | `draw_editor_split_or_single` | Same splits; true multi-cursor UI freer |
| Welcome | empty buffer | `draw_welcome` | Same identity (晴 / shade) + glass |
| Gutter | line no + git + fold + BP | in editor | Same glyphs · **click targets free** |
| Blame panel | Ctrl+B | slide left flame strip | Same open/close; smoother anim |
| Explorer | Ctrl+F | `draw_explorer` | Tree + icons (real icons OK) |
| Ext panel | SPC x | activity + list | Real activity bar materials |
| Terminal | Ctrl+T / full | PTY cells | Native PTY view / better scrollback |
| XLC | `:` | bottom cmd + log | Same; better log UI |
| Search bar | `/` `?` | 1-row | Same; find widget upgrade later |
| Status line | always | badges | Glass status; more room for badges |
| Palette | Ctrl+P / Shift+P | frosted popup | Glass modal |
| Which-key | prefix delay | chord popup | Glass near caret |
| Completions | Insert | popup | Same list; better ranking UI later |
| Hover | K | popup | Rich markdown free |
| Peek | gp | card | Soft shadow free |
| SCM | Ctrl+G | slide-in | Same focus model |
| Git workbench | Ctrl+Shift+G | 3-pane dock | Same tabs; richer graph |
| Settings | Ctrl+, | About/Setting/Pet/Help | Native prefs + glass |
| Preview | Ctrl+Shift+V | wavefront + GFM | **Real images/fonts free** |
| Workspace search | Ctrl+Shift+F | full overlay | Same |
| DAP | panel_open | bottom dock | Same panes; mouse already planned |
| Call hierarchy | gC | modal list | Same |
| Rebase | `:rebase` | modal plan | Same |
| PR review | `:pr` | multi-pane | Same |
| Plugin store | `:plugins` | full screen | Same + web assets |
| Webview | extension | Kitty image | **Real WKWebView** |
| Screensaver | `:ss` | xeifetch | Optional; or skip on desktop |
| Ctx menus | right-click | editor/git | Native menus free |

---

## 1.3 Editor cell model (what Suisei must reproduce)

Per visible row (simplified from `draw_editor`):

1. **Gutter** fixed width (`LINE_NO_WIDTH` = 5): git/fold/BP glyph + line number (absolute or relative)  
2. **Text**: soft-wrap segments; CJK double-width; tab expansion  
3. **Highlight stack**: semantic tokens > tree-sitter query > line fallback  
4. **Overlays on line**: selection, search match, diag underline, inlay, code lens EOL, multi-carets  
5. **Viewport maps**: `screen_row_to_buffer`, `screen_row_visual_base` for mouse  

Suisei `FrameDiff` must carry enough of (1)–(5) that the canvas is a **dumb blitter**.

---

## 1.4 Interaction model (chrome + mouse)

Already in TUI (port hit-tests as UI regions, not as cell math):

- Tab chips, explorer rows, split separators, git tabs/panes/log rows  
- DAP tabs/rows, gutter BP click  
- Double-click word, drag select, wheel → editor / term / preview / xlc  
- Resize explorer / terminal / xlc by drag  

Suisei: same hit-targets, but **pixel rects** + OS cursor affordances.

---

## 1.5 TUI limits → Suisei must unlock

| Limit in TUI | Suisei expectation |
|--------------|-------------------|
| Cell grid only (no sub-cell geometry) | Subpixel / fractional scroll, smooth caret |
| 16/truecolor cells, no real font shaping | Proper fonts, ligatures optional, crisp CJK |
| Kitty graphics for images/webview | Native image + real webview |
| Chord keys / Ctrl+Shift flaky on legacy TTYs | Full key matrix always |
| One “frame” = full terminal redraw discipline | Damage rects, vsync, 120Hz displays |
| Density capped by columns | Adaptive density, multi-window later |
| No OS menus / drag files into chrome easily | Menu bar, Finder drop, Services |
| Glass / blur impossible | Liquid glass / vibrancy |
| Extension webview = screenshot | Live interactive webview |
| One tab strip / one main pane | **Optional infinite canvas** of open files (Driftwm-like) |

**Rule of product:** if xei already has a surface, Suisei has it **in the same place with the same keys**. If xei only *fakes* fidelity (image as Kitty bitmap, webview as PNG), Suisei does the **real** thing.

---

## 1.6 Workspace chrome modes — Tabs vs Infinite Canvas

Suisei has **two ways to arrange open files**. Same Core buffers / same keys for editing; only **how documents are placed on screen** changes.

| Mode | UX | Default |
|------|-----|---------|
| **`Tabs`** | xei-identical: tab bar + one main (+ splits). Muscle memory 1:1 with TUI. | **Yes** (ship default) |
| **`Canvas`** | Driftwm-inspired: open files live as **cards on an infinite 2D plane**; the window is a **camera** (pan / zoom / overview). | Opt-in via Settings |

**Inspiration ([driftwm](https://github.com/malbiruk/driftwm)):**  
Traditional UIs force everything into the screen rectangle (stack or tile). Driftwm puts windows at native size on an infinite canvas and treats the display as a viewport. Suisei borrows that model **for editor documents** (not as a full OS compositor): each open buffer (or focused split group) can be a floating glass card; pan/zoom finds context; snap-together optional later.

**Settings toggle (must exist):**

```text
Settings → Appearance / Workspace
  Document layout:  (•) Tabs   ( ) Infinite canvas
```

- Persist in user config (`workspace.document_layout = "tabs" | "canvas"`).  
- Toggle live without restart; Core keeps the same open-buffer set and focus id.  
- **TUI always behaves as Tabs** — canvas is a Suisei-only unlock (terminal cannot host a free plane).

**Canvas v1 scope (honest):**

| In | Out (later / never v1) |
|----|-------------------------|
| Open files as draggable cards | Full desktop WM / arbitrary OS apps |
| Camera pan + zoom + fit-all / center-focus | Multi-monitor independent cameras (maybe later) |
| Click card → Core focus that buffer | Replacing explorer/activity bar (those stay chrome) |
| Persist card positions per workspace path | Cluster snap groups (nice-to-have S6+) |
| Same editor paint pipeline inside each card | Infinite *scrollable code* as the canvas (no — canvas is **files**, not buffer space) |

**Chrome that stays even in Canvas mode:**  
status line, palette/which-key, explorer/ext panel (optional), status/dap floats — they are **pinned-to-screen** (Driftwm’s “pinned to screen” idea), not floating on the plane. Only **document surfaces** live on the canvas.

**Mental model:**

```
Tabs mode:     [tab chips]  →  one main editor rect
Canvas mode:   infinite plane
                 ┌─────┐  ┌─────┐
                 │ a.rs │  │b.ts │   camera ──▶ window
                 └─────┘  └─────┘
                      ┌──────────┐
                      │ README   │
                      └──────────┘
```

---

## 1.7 Implications for implementation order

1. **Clone chrome geometry first** (tabs / crumbs / body columns / status) — empty editor OK.  
2. **Editor cell pipeline second** — must match gutter + wrap + highlight stack.  
3. **Mode surfaces third** — one by one, same Mode enum / open flags from core.  
4. **Unlock list last** — glass, webview, font shaping, multi-window.  
5. **Canvas mode after Tabs chrome is solid** — second layout engine on the same Core buffers (see §3.9). Do not block S1–S4 on canvas.

Do not design a VS Code–clone layout. Design an **xei-clone layout** with desktop materials, **plus** an optional spatial document mode.

---

## 2. Why a thin face (Swift *or* web) can still win on speed

Electron loses when **every keystroke and paint** is JS/DOM. Suisei does not put the engine in the face.

```
Hot path  → 100% Rust (Core + Compositor + Bridge routing)
Warm path → Rust → FrameDiff → thin face update (FFI or binary IPC)
Cold path → face-only chrome (settings forms, about, glass decoration)
```

| Workload | Owner | Notes |
|----------|--------|------|
| Insert / delete / undo | Rust Core | Already O(n) paste, delta undo |
| Soft-wrap metrics, CJK columns | Rust Compositor | Share metrics with TUI where possible |
| Syntax / semantic tokens | Rust | Spans once; face only blits |
| Scroll / damage rects | Rust | Only dirty lines cross the face boundary |
| LSP / DAP / git child procs | Rust | Async poll like TUI main loop |
| Keymap resolution | Rust | Single table |
| Extension host | Node (ext-host) | Same process model as xei v2 |
| Glass chrome, animations | **Renderer face** | SwiftUI materials *or* CSS glass — not 10MB buffers |
| Extension webviews | **WKWebView island** | Isolated; never on typing path |
| Editor glyph paint | Face GPU (Metal / canvas / wgpu) | Dumb blitter of `PaintLine` |

**Performance budgets (targets, not yet measured):**

| Metric | Target | VS Code (rough) |
|--------|--------|------------------|
| Cold start to editable | &lt; 400ms (no extensions) | 1–3s+ |
| Keystroke → glyph | &lt; 8ms p95 | often 10–20ms+ under load |
| RSS idle, 1 file | &lt; 80MB | 300–600MB+ |
| Open 50k-line file | first paint &lt; 100ms | often multi-hundred ms |

Budgets force design: **no full-document JSON every frame**, no virtual DOM for every cell.  
Renderer **stack choice** (§3.6) must still obey FrameDiff — stack is swappable; contract is not.

---

## 3. Architecture — Core · Compositor · Bridge · Renderer

스이세이 런타임은 네 층으로 고정한다.  
**성능과 효율은 “누가 빠른가”가 아니라 “경계가 새지 않는가”에서 나온다.**

- 경계를 흐리면: 로직이 JS로 새거나, 전체 문서가 IPC로 매 키 전송되거나, 두 곳에 버퍼가 생긴다.  
- 경계를 고정하면: 핫 패스는 프로세스 안 Rust 호출, IPC는 damage만, Renderer는 블리트만.

```
                         ┌─────────────────────────────────────┐
                         │  OS / WebView / DOM / Trackpad      │
                         └──────────────────┬──────────────────┘
                                            │ raw events
                                            ▼
┌──────────────────────────────────────────────────────────────────────────┐
│  BRIDGE  — 유일한 바깥 경계                                               │
│  normalize · route · serialize · platform · schedule                      │
└───────────────┬──────────────────────────────────────────▲───────────────┘
                │ InCommand (in-proc)                      │ FrameDiff bytes
                ▼                                          │ (IPC only here)
┌───────────────────────────┐   read-only view    ┌────────┴───────────────┐
│  CORE                     │ ──────────────────▶ │  COMPOSITOR            │
│  문서·모드·키맵 진실       │   AppSnapshot       │  배치·damage·히트       │
│  mutate via dispatch only │ ◀────────────────── │  Scene / FrameDiff     │
└───────────────────────────┘   (never mutates)   └────────────────────────┘
                ▲
                │  (Renderer never reaches here)
                │
         ┌──────┴──────┐
         │  RENDERER   │  ← last Scene / FrameDiff mirror only
         │  paint only │
         └─────────────┘
```

**프로세스 / 호출 경계 (성능의 핵심):**

| 경계 | 종류 | 허용 비용 |
|------|------|-----------|
| Core ↔ Compositor | **같은 프로세스, 함수 호출** | 제로카피 뷰 / 공유 참조. IPC 금지. |
| Bridge ↔ Core/Compositor | **같은 프로세스, 함수 호출** | 얇은 라우팅. 비즈니스 if 금지. |
| Bridge ↔ Renderer | **Face boundary** (FFI callback 또는 IPC) | **유일한 “나가기” 지점.** damage only. 스택별 전송 수단은 §3.6. |
| Renderer ↔ Core | **금지** | 어떤 경로로도 직접 호출/import 불가. |

xei TUI의 `ui::draw` ≈ **Compositor + (ratatui) Renderer** 가 한 파일에 섞인 것.  
스이세이는 둘을 가르고, **Core는 TUI와 완전 공유**, **IPC는 Bridge↔Renderer 한 곳만**.

---

### 3.0 한 줄 + 소유 권한

| 층 | 한 줄 | **Sole owner of** | **May read** | **Must not** |
|----|--------|-------------------|--------------|--------------|
| **Core** | 문서와 모드의 진실 | buffers, cursor, folds, Mode, keymap tables, LSP/DAP/git state, open-file set, focus buffer id, user config values | nothing from UI | pixels, rects, DOM, Tauri, font rasters, glass |
| **Compositor** | App → 그릴 수 있는 Scene | layout tree, hit regions, soft-wrap of *visible* range, `PaintLine`s, damage gen, canvas camera/card rects | Core via immutable snapshot / `&App` getters | `buffer.insert`, key resolution, IPC, DOM |
| **Bridge** | 바깥↔안 어댑터 + 스케줄러 | IPC sockets, OS handles, tick clock, last-sent gen, input device raw state | routes only | layout policy, buffer text cache, theme business rules |
| **Renderer** | Scene을 픽셀로 | GPU/canvas resources, CSS glass tokens, last applied `FrameDiff` mirror | Scene/FrameDiff only | document rope, Vim state, “smart” key handling, inventing layout |

**소유 판정 한 줄 테스트**

1. “이 값이 틀리면 문서가 틀린 건가?” → **Core**  
2. “이 값이 틀리면 화면 배치/히트만 틀린 건가?” → **Compositor**  
3. “OS/프로세스/직렬화 없으면 이 값이 의미 없나?” → **Bridge**  
4. “GPU/CSS 없으면 이 값이 의미 없나?” → **Renderer**

모호하면 **위로 올리지 말고** (Renderer→Core 금지), **Core 쪽으로 흡수하거나 Compositor에 두고 읽기 전용으로 노출**.

---

### 3.1 Boundary contracts (네 모서리)

경계를 **타입 + 방향 + 빈도**로 고정한다. 새 기능은 이 표에 행을 추가할 수 있을 때만 넣는다.

#### A. Bridge → Core  (`InCommand`)

| 명령 종류 | 예시 | Core 반응 |
|-----------|------|-----------|
| 정규화 키 | `Key(KeyEvent)` | `App::dispatch` — **유일한 키 해석 입구** |
| 정규화 마우스(의미) | `Mouse { target, btn, pos_in_target }` | 선택/클릭/스크롤 등 core 의미 액션 |
| 고수준 파일 | `OpenPath`, `Save`, `CloseBuffer` | 버퍼 집합 변경 |
| 설정 | `SetConfig { key, value }` | config + 필요 시 version++ |
| 포커스 | `FocusBuffer(id)` | Tabs/Canvas 공통 |
| 틱 | `Tick(dt)` | lsp/dap/git/hooks poll |

Bridge는 **키 문자열을 해석하지 않는다.**  
`Ctrl+S` → save 같은 매핑은 **Core keymap**만.

#### B. Bridge → Compositor  (`ShellCommand`) — Core를 거치지 않는 것

Core 문서 진실과 무관한 **셸 배치**만.

| 명령 | 예시 |
|------|------|
| 뷰포트 | `Resize { css_w, css_h, cell_px, dpr }` |
| 캔버스 카메라 | `CanvasPan`, `CanvasZoom`, `CanvasFitAll` |
| 카드 배치 | `CardMove { id, world_rect }`, `CardResize` |
| 애니 시각 | `SetSlideT { panel, t: 0..1 }` (값은 Compositor가 소유, Bridge는 입력 전달) |

**판정:** Core `version`을 안 바꿔도 화면이 바뀌면 → ShellCommand.  
바꾸면 → InCommand.

#### C. Core → Compositor  (in-process, **read-only**)

Compositor는 Core를 **mutate 하지 않는다.**

```text
Compositor::compose(app: &App, shell: &ShellState, prev: &Scene) -> FrameDiff
```

읽는 것 (예):
- buffer text / line count / cursor / selection / folds  
- mode + panel open flags  
- diagnostics, git gutter bits, semantic tokens (core가 캐시한 것)  
- open tabs order, dirty flags, status fields  
- `app.content_version` / `ui_version` (damage 힌트)

쓰지 않는 것:
- 어떤 `app.* =` 도 금지. 필요하면 Bridge가 Core 명령을 다시 넣는다.

#### D. Compositor → Bridge → Renderer  (`FrameDiff` only on wire)

**IPC에 실을 수 있는 것은 FrameDiff(와 드물게 풀 Scene 스냅샷)뿐이다.**

```text
FrameDiff {
  from_gen, to_gen,
  chrome: Option<ChromeScene>,     // 구조/라벨 변경 시만
  canvas: Option<CanvasDamage>,    // 카메라·카드 rect
  panes:  [PaneDamage],            // 보이는 pane/card의 줄 범위
  floats: FloatUpdate,             // palette 등
  hits:   Option<[HitRegion]>,     // 히트맵 변경 시 (Bridge가 캐시)
}
```

| 규칙 | 이유 |
|------|------|
| 기본: dirty line range만 | 키스트로크 IPC ≪ 전체 파일 |
| theme/resize/fold-all: full pane 허용 | 드묾 |
| 50k-line 파일을 JSON 문자열 배열로 매 키 전송 금지 | 예산 붕괴 |
| Renderer는 gen 단조 증가만 적용 | 오래된 diff 드롭 |

#### E. Renderer → Bridge  (raw만)

| 이벤트 | Bridge가 하는 일 |
|--------|------------------|
| keydown/keyup, IME composition | → `KeyEvent` 정규화 → Core |
| pointer + modifiers | hit-test(Compositor hits 캐시) → `Mouse` 또는 `ShellCommand` |
| wheel / pinch | target이 editor면 Core scroll; canvas 빈 공간이면 ShellCommand |
| resize / DPR | `ShellCommand::Resize` |
| menu / file drop | `OpenPath` 등 InCommand |
| settings widget change | `SetConfig` |

Renderer는 hit id를 **스스로 해석해 비즈니스 동작을 결정하지 않는다.**  
`"tab:3 클릭"` → Bridge/Compositor 히트 결과 → Core `FocusBuffer` / close.  
Svelte `on:click={() => buffer = ...}` **금지**.

---

### 3.2 관심사 소유 매트릭스 (완전 표)

| 관심사 | Core | Compositor | Bridge | Renderer |
|--------|:----:|:----------:|:------:|:--------:|
| Rope / undo / Vim ops | **W** | — | — | — |
| Cursor, selection, multi-cursor | **W** | R | — | paint |
| Folds, jumplist, registers | **W** | R | — | — |
| Mode enum, panel open flags | **W** | R | — | chrome mirror |
| Keymap resolve | **W** | — | normalize only | — |
| LSP/DAP/git process state | **W** | R (gutter/status bits) | spawn/supervise | — |
| Open buffer set + focus id | **W** | R | route focus cmds | — |
| User config values | **W** | R | persist IO | forms (cold) |
| Soft-wrap of **visible** lines | — | **W** | cell metrics in | — |
| Gutter glyphs + line assembly | — | **W** | — | blit |
| Syntax spans (from tokens) | tokens **W** | assemble **W** | — | blit colors |
| Tab bar / explorer / status **geometry** | — | **W** | resize in | paint |
| Canvas camera + card world rects | — | **W** | gestures in | paint plane |
| Hit regions | — | **W** | hit-test use | optional debug |
| Damage / scene gen | — | **W** | last-sent gen | apply gen |
| IPC encode/decode | — | produces struct | **W** bytes | decode |
| OS menu, dialog, vibrancy, clipboard | — | — | **W** | trigger only |
| Font raster, ligatures, glass blur | — | — | font path maybe | **W** |
| Local document copy | **forbid** | **forbid** | **forbid** | **forbid** |
| Second keymap table | **forbid** | **forbid** | **forbid** | **forbid** |

**W** = sole writer · **R** = read-only · **—** = no access · paint/blit = pixels only

---

### 3.3 Core

**Current location:** `suisei-core` (a fork of `xei-core`). The desired Core
boundary below still applies, but references to a shared `xei-core` are
historical planning assumptions.

**Sole writer:**
- 버퍼, undo, Vim ops, folds, multi-cursor  
- Mode / 패널 open 플래그  
- LSP · DAP · git · hooks · session 의미 상태  
- keymap 테이블과 `dispatch`  
- config 값, open buffers, focus id  

**공개 API (S0 필수):**

| API | 계약 |
|-----|------|
| `KeyEvent` | 셸 중립 키 표현 |
| `App::dispatch(KeyEvent) -> DispatchResult` | 사이드이펙트는 App 안만; UI 모름 |
| `App::command(AppCommand) -> …` | 마우스/메뉴/파일 등 비키 입구 (키와 동일 상태머신으로 합류) |
| `App::tick(dt)` | poll 일원화 |
| `App::content_version` / `ui_version` | Compositor damage 힌트 (단조 증가) |
| getters | Compositor가 읽는 불변 뷰 |

**금지 (하드):**
- `ratatui`, DOM, Tauri, 픽셀 좌표, “탭 칩 폭 px”  
- FrameDiff / IPC 타입 의존  
- “이 키를 화면에서 어떻게 그릴지” 결정  

TUI는 같은 `dispatch`를 호출한다. 셸별 키 테이블 분기 = 아키텍처 실패.

---

### 3.4 Compositor

**위치:** `suisei::compositor` — **core 밖, renderer 밖, 같은 Rust 프로세스.**

xei `draw()`의 레이아웃·z-order·에디터 줄 조립을 **순수 함수에 가깝게** 재현.  
출력은 픽셀이 아니라 **Scene / FrameDiff**.

**Sole writer:**
- §1.1 스크린 맵 → 노드 트리 (Tabs) 또는 `CanvasScene` (Canvas)  
- soft-wrap + gutter + highlight stack → `PaintLine` (보이는 범위만)  
- damage 범위, `gen`  
- hit regions  
- canvas camera / card rects / snap (shell state)

**입력:** `&App` + `&ShellState` + previous `Scene`  
**출력:** `FrameDiff` (또는 최초 `Scene`)

**금지 (하드):**
- `App` 필드 mutate  
- keymap / Vim 의미 해석  
- DOM·Canvas·wgpu 호출  
- 네트워크·파일 다이얼로그  
- “예쁘게” (blur radius, spring) — 수치 토큰은 Renderer; Compositor는 **논리 진행도 0..1** 또는 **논리 rect**만

**타입 스케치:**

```text
DocumentLayout = Tabs | Canvas

ShellState {                    // Compositor-owned; not Core
  viewport: { css_w, css_h, cell_px, dpr },
  layout: DocumentLayout,
  panel_widths: { explorer, terminal, … },
  canvas: Option<CanvasState>,  // camera, cards[]
  anim: { explorer_t, … },      // 0..1
}

Scene {
  gen: u64,
  layout: DocumentLayout,
  chrome: ChromeScene,
  main: MainScene,
  canvas: Option<CanvasScene>,
  floats: [FloatScene],
  hits: [HitRegion],
  editor_frames: [PaneFrame],
}

PaneFrame {
  pane_id,
  buffer_id,
  damage: LineRange | Full,
  lines: [PaintLine],           // only damaged or visible band
  carets: [Caret],
  maps: ScreenToBuffer,         // mouse → core coords
}

FrameDiff { from_gen, to_gen, chrome?, canvas?, panes[], floats, hits? }
```

**효율 규칙:**
1. `content_version` 불변 + scroll 불변 + shell 불변 → **FrameDiff::empty** (IPC 스킵).  
2. 키 입력 한 글자 → 보통 **1 pane × 1~N lines**.  
3. Canvas: viewport와 교차하는 카드만 `editor_frames` 생성.  
4. Overview zoom-out: 타이틀 칩 축약; full paint pipeline 금지.  
5. Compositor는 매 틱 full Scene rebuild를 **하지 않는다** — prev Scene에 패치.

---

### 3.5 Bridge

**위치:** Tauri 호스트 + 최소 입력 어댑터.  
**역할 한 줄:** *번역기 + 스케줄러 + 플랫폼 손.* 두뇌 아님.

**Sole writer:**
- OS/웹 이벤트 정규화  
- IPC 바이트 송수신, last-sent `gen`  
- ext-host / child process 수명 (의미 상태는 Core로 흡수)  
- 메뉴·다이얼로그·vibrancy·클립보드 호출  
- 틱: “입력 직후 즉시 compose” + “display link / ≤16ms idle poll”

**파이프라인 (고정 순서):**

```text
1. recv raw event
2. normalize
3. if needs hit-test → use Compositor hits cache (no layout invent)
4. route:
     document meaning  → Core (InCommand)
     shell geometry    → Compositor ShellState update
5. if Core mutated or ShellState dirty:
     FrameDiff = Compositor::compose(&app, &shell, &prev)
6. if FrameDiff non-empty && gen > last_sent:
     serialize → emit to Renderer
7. Renderer never called from here except via IPC event
```

**금지 (하드):**
- `if key == 's' && ctrl { save() }` 같은 **두 번째 키맵**  
- 버퍼 텍스트 캐시  
- “explorer 폭 기본  halve if …” 같은 **레이아웃 정책** (그건 Compositor)  
- Renderer store를 읽어 비즈니스 결정  

**얇을수록 빠르다.** Bridge 비대 = 이전 suisei 사망 원인과 동일 패턴.

---

### 3.6 Renderer — 역할 (스택 중립)

**Sole writer:**
- GPU/view 리소스, 글리프 캐시  
- glass / material 시각  
- last `FrameDiff` mirror — **문서 진실 아님**

**한다:**
- `ChromeScene` → 네이티브 또는 DOM 크롬  
- `PaneFrame` → 에디터 서피스 blit (gutter, spans, caret)  
- Compositor가 준 rect / 0..1 t 로 애니메이션 표현  
- a11y, reduced motion  

**금지 (하드) — 스택과 무관:**
- document rope / line 배열을 “내 상태”로 보관 후 자체 편집  
- 자체 Vim / 키바인딩 테이블  
- LSP 요청, git 호출  
- Core 타입 직접 조작  
- 레이아웃 재계산 (탭 폭을 face가 다시 셈 → Compositor와 드리프트)

**cold path 예외:** Settings 폼 등 비문서 UI는 face 로컬 폼 state 가능.  
Apply 시에만 `SetConfig` → Core.

**TUI 한계 돌파 지점:**  
서브픽셀 스크롤, 폰트 shaping, 리얼 이미지/webview, 완전 키 매트릭스, 120Hz, liquid glass.

---

### 3.6.1 Renderer 기술 선택 — UI 풀 Rust는 하지 않는다

**결정 (잠금):** Core · Compositor · Bridge = **Rust**.  
**Renderer UI 크롬 전체를 Rust(egui/iced/gpui 등)로 짜지 않는다.**  
이유: IDE 크롬(글래스, 메뉴, a11y, 트랙패드, 설정, 웹뷰 섬)을 Rust UI 툴킷으로 재발명하는 비용이 엔진 이득을 잡아먹음. 이전 suisei 교훈은 “웹이 느려서”가 아니라 **경계 붕괴**였음.

Renderer는 **교체 가능한 face**. 엔진이 고정하는 것은 오직:

```text
Bridge  ←raw events—  Renderer
Bridge  —FrameDiff→  Renderer
```

스택을 바꿔도 Core/Compositor/키맵/버퍼는 不动.

---

### 3.6.2 후보 비교 (macOS v1)

| | **A. Swift face** | **B. Tauri + Svelte face** | **C. 하이브리드** |
|--|-------------------|---------------------------|-------------------|
| **크롬** | SwiftUI / AppKit | Svelte DOM | SwiftUI 크롬 |
| **에디터 페인트** | Metal / Core Text (또는 MTKView) | Canvas2D → WebGPU | Metal |
| **Bridge↔Face** | **in-process FFI** (UniFFI / cxx / cdylib 콜백) | Tauri IPC (이벤트/채널) | FFI + WKWebView 섬 |
| **Liquid glass** | **진짜** materials / vibrancy | CSS + window effects 근사 | 진짜 |
| **RSS / 시작** | 웹뷰 셸 없음 → 예산에 유리 | 메인 UI가 WKWebView → 베이스라인 높음 | 크롬 네이티브, 확장만 웹 |
| **확장 webview** | WKWebView 섬 | Tauri webview / 추가 WKWebView | WKWebView 섬 |
| **크로스 플랫폼** | Linux/Win은 **다른 face** 또는 나중 | 같은 face 재사용 쉬움 | mac face + 나중 web face |
| **개발 속도 (크롬)** | Swift 숙련 필요, 미리보기 좋음 | 웹 이터레이션 빠름 | 두 스택 |
| **Canvas 모드** | Metal 평면 + 카드 뷰 자연스러움 | CSS transform / canvas 월드 | Metal |
| **위험** | Rust↔Swift 브릿지 초기 비용; CI에 Xcode | 글래스가 “웹 앱”처럼 보임; IPC 규율 필수 | 복잡도 최고 |

**풀 Rust UI (iced/egui/gpui):** 엔진과 한 언어인 점은 매력이나, macOS 글래스·메뉴·확장 웹뷰·생산성을 한꺼번에 만족시키기 어렵다고 보고 **v1 비권장** (Zed-gpui 수준 투자는 별 제품).

---

### 3.6.3 권장 — macOS v1 = **Swift face** (기본안)

**권장 기본안: A — SwiftUI 크롬 + Metal(또는 Core Text 기반) 에디터 서피스.**

| 왜 | |
|----|--|
| 제품 기둥 | “Looks native / liquid glass”를 **근사하지 않고** 달성 |
| 성능 예산 | 메인 셸에 Chromium/대형 웹 런타임 없음 → cold start·RSS에 유리 |
| 경계 | Rust cdylib이 프로세스 안에서 `compose` 후 **FrameDiff를 콜백/공유버퍼로** 넘김 → Tauri IPC보다 핫 패스 단순 |
| 확장 | VS Code webview만 **WKWebView 섬** (타이핑 경로 밖) |
| 교체 가능성 | FrameDiff 스키마만 지키면 나중에 Linux용 Tauri face 추가 가능 |

**Bridge 형태 (Swift 안):**

```text
suisei-app (Swift)          suisei engine (Rust cdylib)
  NSApp / SwiftUI               Core + Compositor + Bridge-logic
  key/mouse ──FFI──▶            normalize → dispatch → compose
  Metal view ◀─FrameDiff─       damage lines / chrome struct
  WKWebView (ext only)          ext-host stdio still Rust-side
```

- “Bridge” 로직(정규화·라우팅·틱)은 **여전히 Rust**.  
- Swift는 **Renderer + OS 호스트** (창, 메뉴, 파일 다이얼로그, vibrancy).  
- 직렬화: 같은 프로세스면 **바이너리 구조체 / 공유 버퍼** 우선; JSON은 콜드만.

**Plan B — Tauri + Svelte:**  
웹 이터레이션·향후 멀티 OS를 v1부터 최우선으로 둘 때.  
조건: FrameDiff 바이너리, 에디터는 DOM 텍스트 금지, 글래스는 window effects + 자제된 CSS.  
이전 scaffold는 **코드 재사용 없이** 경계 규율만 계승.

**Plan C — 하이브리드:**  
A와 동일하되 설정/스토어 등 일부 콜드 UI만 작은 WKWebView. v1에 필수는 아님.

---

### 3.6.4 스택별 “나가기” 경계 (같은 계약, 다른 운반)

| | Swift (권장) | Tauri + Svelte (B) |
|--|--------------|---------------------|
| 이벤트 인 | NSEvent → Rust FFI | DOM key/pointer → IPC → Rust |
| FrameDiff 아웃 | FFI callback / ring buffer | Tauri event / channel (binary) |
| 틱 | `CVDisplayLink` / `CADisplayLink` → Rust tick | requestAnimationFrame 또는 Rust 타이머 |
| 메뉴/다이얼로그 | AppKit/SwiftUI | Tauri plugin / OS API |
| 에디터 서피스 | Metal blit of `PaintLine` | canvas 2d / WebGPU blit |
| 확장 webview | `WKWebView` | Tauri webview / `WKWebView` |

**어느 쪽이든 금지:** face 안에서 버퍼 편집, face 키맵 테이블, face→Core 타입 직행.

---

### 3.6.5 잠금 vs 열린 결정

| 잠금 | 아직 고를 수 있음 |
|------|-------------------|
| UI 풀 Rust 안 함 | Swift(A) vs Tauri(B) — **기본 권장 A** |
| FrameDiff가 face 유일 입력 | 에디터: Metal vs Core Text vs (B) canvas |
| Core/Compositor/키맵 = Rust | UniFFI vs 수동 C ABI |
| 확장 풀 페이지 = WKWebView 섬 | S1을 Swift hello vs Tauri hello로 시작할지 |

S0(`dispatch` 추출)은 스택과 **무관** — 먼저 진행.  
S1 scaffold만 face 선택에 갈림.

---

### 3.7 데이터 흐름 (세 경로)

**핫 — 키 한 번 (예산 &lt; 8ms p95의 대상):**

```text
keydown
  → Bridge normalize → KeyEvent
  → Core::dispatch          // in-proc, mutate buffer/mode, version++
  → Compositor::compose     // in-proc, damage lines only
  → Bridge handoff FrameDiff   // FFI callback or binary IPC (§3.6.4)
  → Renderer blit dirty rects
```

**웜 — 스크롤 / 리사이즈:**

```text
wheel/resize
  → Bridge
  → Core scroll  or  ShellState viewport
  → Compositor reproject visible band
  → FrameDiff ≈ visible range
  → Renderer
```

**콜드 — 설정 / 글래스 / 메뉴:**

```text
Settings toggle document_layout
  → Bridge SetConfig
  → Core config
  → Compositor branch Tabs|Canvas (full chrome FrameDiff)
  → Renderer rebuild chrome (non-typing path OK)
```

---

### 3.8 경계 위반 = 리뷰 거절 (체크리스트)

1. **Renderer → Core 직접 호출/import**  
2. **Core가 픽셀·DOM·Tauri·FrameDiff를 앎**  
3. **Compositor가 App을 mutate**  
4. **Bridge에 두 번째 keymap 또는 버퍼 로직**  
5. **전체 파일을 매 키 IPC** (damage 아닌 full text)  
6. **Renderer가 문서 텍스트 SoT를 보관**  
7. **Core↔Compositor 사이 IPC/JSON** (같은 프로세스여야 함)  
8. **새 UI 면**을 넣을 때 Mode/플래그(Core) · 배치(Compositor) · 스킨(Renderer) 분리가 안 됨  
9. **Canvas 카드 좌표를 Core 버퍼 구조체에 심음** (좌표는 ShellState/Compositor)  
10. **히트 없이** Renderer가 `buffer_id`를 하드코딩해 포커스 변경

---

### 3.9 Tabs ↔ Canvas — 같은 경계 위

| 관심사 | Core | Compositor | Bridge | Renderer |
|--------|------|------------|--------|----------|
| Open buffers / focus / dirty | **W** | R | route | paint title |
| `document_layout` config | **W** | branch | SetConfig | toggle cold UI |
| Tab chip geometry | — | **W** | — | paint |
| Card world rects + camera | — | **W** | gestures | plane + cards |
| Typing / Vim / LSP | **W** | pane damage | key route | blit |
| Palette / which-key | mode **W** | float z | — | glass modal |
| Card position session | session blob R/W via Bridge IO | cards load | persist file | — |

**Canvas 입력:**
- 카드 **안** + 에디터 포커스 → `KeyEvent` → Core (xei 동일)  
- 카드 **밖** / Mod+gesture → `ShellCommand` → Compositor only; 포커스 변경 시만 Core `FocusBuffer`  
- 트랙패드 우선; 편집 맵과 충돌 피하려면 canvas 내비는 Mod 접두

**전환:**  
Tabs↔Canvas 모두 **같은 open-buffer set**. Compositor만 배치 엔진 교체.  
Tabs→Canvas: 탭 순서로 카드 seed. Canvas→Tabs: 카드 위치 세션 저장, 탭 순서는 Core open order.

```
suisei/src/compositor/
  layout_tabs.rs
  layout_canvas.rs
  canvas_camera.rs
  canvas_snap.rs      # later
```

---

### 3.10 디렉터리 매핑 (= 경계의 물리적 강제)

```
suisei-core/                       # CORE only (currently forked from xei-core)
  src/{app,buffer,keymap,…}

suisei-engine/                     # Rust: runtime + BRIDGE logic + COMPOSITOR
  src/
    runtime.rs                     # owns App + ShellState
    bridge/
      face.rs                      # FrameDiff handoff (FFI or IPC) — ONLY exit
      input.rs
      route.rs
      platform_macos.rs
      tick.rs
    compositor/
      scene.rs, shell_state.rs, compose.rs, …
  # crate-type: cdylib + rlib  (Swift가 링크 / 또는 Tauri가 링크)

suisei-app/                        # RENDERER host — pick ONE for v1
  # --- Plan A (recommended): Swift ---
  SuiseiApp/                       # Xcode / SPM
    Chrome/                        # SwiftUI tabs, explorer chrome, status
    Editor/                        # Metal (or Core Text) blit of PaneFrame
    WebViewIsland/                 # extension WKWebView only
    BridgeFFI/                     # generated UniFFI / C headers

  # --- Plan B (alt): Tauri + Svelte ---
  src-tauri/                       # thin host linking suisei-engine
  ui/                              # Svelte chrome + canvas editor
```

**의존 방향:**

```text
suisei-core       ←  suisei-engine
suisei-engine     →  (no Swift, no Svelte source)
suisei-app/Swift  →  engine C/FFI API only
suisei-app/web    →  engine via Tauri commands/events only
face         -X->  suisei-core types
compositor   -X->  SwiftUI / DOM
suisei-core  -X->  suisei / tauri / ratatui / Swift
```

`ratatui`는 **xei TUI 크레이트**에만.  
TUI Renderer = ratatui, Suisei Renderer = Swift 또는 Svelte — 둘 다 Core 밖.

Workspace members, build scripts and the Swift application already exist. The
directory diagram is aspirational only where it differs from the current tree.

---

### 3.11 xei TUI 대응

| xei today | Suisei layer |
|-----------|----------------|
| `suisei_core::App` | Core |
| `xei/src/event.rs` 키 해석 | Core `dispatch` |
| `xei/src/event.rs` crossterm read | Bridge input |
| `xei/src/ui.rs` layout / z-order | Compositor |
| `draw_editor` line assembly | Compositor `editor_frame` |
| ratatui widgets | Renderer (TUI) / Swift Metal or Svelte canvas (Suisei) |
| main loop poll | Bridge tick → Core::tick → compose |

TUI는 Compositor를 **공유하지 않아도 된다** (초기).  
공유 강제 지점은 **Core `dispatch` + 버퍼 진실**.  
레이아웃 공유는 이득이 확실할 때 (soft-wrap 순수 함수 추출 등) 단계적으로.

---

### 3.12 왜 이 경계가 성능·효율을 동시에 잡는가

| 목표 | 경계가 하는 일 |
|------|----------------|
| 키스트로크 &lt; 8ms | Core+Compositor in-proc; IPC는 dirty lines |
| RSS / 큰 파일 | Renderer·IPC에 full rope 없음; visible band만 compose |
| 키맵 1소스 | Bridge/Renderer에 키 해석 없음 → 드리프트 불가 |
| 글래스·120Hz | face(Swift materials / CSS) 자유; Core 오염 없음 |
| 확장 | 새 면 = Core 플래그 + Compositor 노드 + Renderer 스킨, 표에 행 추가 |
| 디버그 | FrameDiff gen·payload size 계측 한 곳 (Bridge) |

**한 문장:**  
Core는 생각하고, Compositor는 배치하고, Bridge는 나르고, Renderer는 찍는다.  
**각자 한 일만 하면** 빠르고, **한 일만 해서** 싸게 유지된다.

---

## 4. Keybinding parity

1. Extract key tables from `xei/src/event.rs` into `xei-core/src/keymap/` (or generate from one declarative source).
2. Both shells feed normalized events:
   - mods: ctrl/alt/shift/meta
   - code: char / f-key / esc / tab / …
3. Golden tests: same sequence → same `App` state hash (buffer text + cursor + mode).
4. macOS: `Meta` maps to the same logical bindings as TUI `Super`/Cmd paths already used in xei.

Which-key / leader (`Space`) must work identically; UI may render the chord popup as glass cards instead of ratatui blocks.

---

## 5. Editor surface (performance-critical)

### 5.1 Data path

```
App.buffer + highlight stack
        ↓  project (Rust)
EditorFrame {
  first_line, scroll,
  lines: [ { row, spans: [(text, style_id)], carets, bp_gutter } ],
  damage: LineRange | Full,
}
        ↓  face boundary (FFI or IPC, damage only)
face blits glyphs (Metal / Core Text / canvas)
```

### 5.2 Editor paint backend (face-specific; S2)

| Face | v1 paint | Later |
|------|----------|-------|
| **Swift (default)** | Metal textured quads **or** Core Text line draws from `PaintLine` | Atlas cache, ligatures optional |
| **Tauri+Svelte (B)** | Canvas 2D blit | WebGPU if budgets miss |

Never put Tree-sitter or full reflow in the face. Metrics stay Rust Compositor.

### 5.3 CJK / multi-width

Reuse core `buffer_col_to_screen_col` / width rules so TUI and desktop never disagree.

---

## 6. UI / liquid glass

### 6.1 Layout language (match xei)

- Tab bar · breadcrumbs · editor · optional side explorer · bottom panel (term/debug/xlc) · status
- Modes reflected in status badge (NORMAL / INSERT / DEBUG / …)
- Palette, which-key, peek as floating glass layers

### 6.1.1 기능 추가 속히 요망 — 편집기 경로 헤더의 pane 컨트롤

**우선순위: Urgent / P1 daily-driver.**

여기서 말하는 “헤더”는 윈도우 최상단의 문서 탭 바가 아니다. 각 편집기
pane 안에서 현재 파일의 경로를 `project › src › file.rs`처럼 보여주는
breadcrumb/jump-bar 행이다.

각 pane 헤더의 오른쪽 끝에는 다음 두 컨트롤을 둔다.

```text
[ current › file › path ] [ outline ]                    [ split ▾ ] [ × ]
```

- **분할 버튼 `split ▾`**
  - pane 수와 관계없이 항상 보인다.
  - 클릭하면 버튼에 anchor된 작은 menu/popover가 열린다.
  - 필수 명령은 `위로 분할`과 `아래로 분할`이다.
  - 명령은 버튼이 속한 pane을 기준으로 실행한다. 다른 pane이 focused여도
    잘못된 pane을 분할하면 안 된다.
  - 실제 분할은 기존 `Split Editor`의 문서·커서·스크롤 보존 규칙을
    공유하며 별도의 임시 레이아웃 상태를 만들지 않는다.
  - 최대 pane 수에 도달하면 명령을 disabled 처리하고 이유를 help로
    설명한다. 클릭 뒤 상태 표시줄에서만 실패를 알리는 방식은 쓰지 않는다.

- **닫기 버튼 `×`**
  - 두 개 이상의 pane이 있을 때는 hover와 무관하게 항상 보인다.
  - pane이 하나뿐일 때는 disabled 버튼이나 빈 자리로 남기지 않고 완전히
    숨긴다.
  - 클릭하면 버튼이 속한 pane만 닫는다. 문서의 전역 tab/buffer를 닫는
    기능이 아니다.
  - 닫힌 pane이 보던 문서는 상단 문서 탭 바에 남아야 하며, unsaved
    buffer를 소리 없이 버리면 안 된다.

이 컨트롤은 단순 편의 기능이 아니다. 사용자가 분할 레이아웃을 눈으로
보면서 해당 pane을 직접 추가·제거하는 주 조작 경로다. Editor 메뉴나
키보드 명령만으로 숨겨 두지 않는다.

DoD:

1. 1-pane: `split ▾`는 항상 보이고 `×`는 존재하지 않는다.
2. 2–4-pane: 모든 pane 헤더에 `split ▾`, `×`가 hover 없이 보인다.
3. `위로 분할`·`아래로 분할`은 클릭한 헤더의 stable pane ID를 대상으로
   정확한 위치에 새 pane을 만든다.
4. `×`는 클릭한 pane만 닫고 남은 split tree, focus, cursor, scroll,
   document tab을 보존한다.
5. 빠른 연속 클릭과 layout fold/unfold 뒤에도 stale pane index를
   참조하지 않는다.
6. 두 버튼은 최소 24×24pt hit target과 `Split editor` / `Close pane`
   접근성 label·help를 가진다.
7. 마우스, VoiceOver action, 키보드 메뉴 경로가 동일한 engine command를
   호출한다.

### 6.1.2 Terminal pane과 Terminal 탭의 identity — 의도된 동작

`Ctrl+Shift+T`는 새 split이나 별도 macOS window를 만드는 명령이 아니다.
focused editor pane의 content를 제자리에서 Terminal로 전환한다. pane
개수와 split tree의 shape는 바뀌지 않는다.

이 전환과 함께 상단 문서 탭 바에 `Terminal` 탭을 만드는 것은 **의도된
기능**이다. Terminal을 임시 overlay로 취급하지 않고 tab identity를
부여해야 다음 규칙이 하나의 모델로 맞아떨어진다.

- 문서 pane과 Terminal pane이 같은 split tree의 leaf가 된다.
- Terminal pane이 포함된 구성을 layout tab으로 fold할 수 있다.
- layout tab을 떠났다가 돌아오거나 unfold해도 같은 Terminal leaf와
  shell session을 복원할 수 있다.
- Terminal 탭의 선택·닫기·그룹 표시는 일반 document/layout tab의
  stable-ID 라우팅을 공유한다.
- Terminal로 교체된 pane의 기존 문서는 문서 탭으로 계속 접근 가능하며
  unsaved buffer를 잃지 않는다.

따라서 `Terminal` 탭의 생성 자체를 중복 탭이나 창 생성 오류로 수정하면
안 된다. 고칠 대상은 pane/tab identity가 어긋나는 경우, shell session이
layout fold/unfold에서 유실되는 경우, 또는 실제 pane 수가 바뀌는 경우다.

### 6.2 macOS materials

- **Swift (default):** transparent titlebar + real `NSVisualEffectView` / SwiftUI materials  
- **Tauri (B):** window effects + restrained CSS blur (approximation)  
- Sidebar / floating panels: blur + thin border + shadow  
- Prefer **system fonts** (SF Pro) for chrome; **editor font** user-configurable (JetBrains Mono / SF Mono)  
- Respect reduced-transparency accessibility  

### 6.3 Theme

Core theme tokens → face (Swift asset catalog / semantic colors, or CSS variables on Plan B) so sakura/ocean/… match TUI hues where possible, with glass-specific elevation on top.

---

## 7. Extensions

Reuse `docs/PLUGIN-RESEARCH.md` and `xei-ext-host` / `xei-ext-host-js`.

| Stage | Suisei |
|-------|--------|
| Declarative | themes, snippets, grammars — load in Rust, no JS |
| Host | same multi-extension host as xei v2 |
| Providers | diagnostics, hover, definition, code actions → core already maps LSP; extensions add extra |
| Webview | **Suisei only** full fidelity; never block typing path |

Suisei must not invent a second plugin format.

---

## 8. IPC contract (sketch)

**Commands (UI → Rust):**  
`dispatch_key`, `dispatch_mouse`, `open_path`, `menu_action`, `resize_editor(cols, rows, cell_px)`, `tick`.

**Events (Rust → UI):**  
`chrome_updated(UiChrome)`, `frame(FrameDiff)`, `notify`, `ext_message`, `quit_request`.

**FrameDiff rules:**
- Default: only changed line range
- Full frame on theme change, resize, fold-all
- Cap IPC payload; never send whole 50k-line file as one JSON array of strings every keystroke — use rope-backed line slices / binary packing when needed

---

## 9. Phased delivery

### S0 — Clean slate & contracts  ← **done (core path)**

- [x] Delete legacy `suisei/` scaffold  
- [x] This design doc + Core/Compositor/Bridge/Renderer boundaries  
- [x] Face = **Swift** locked (§3.6.3, decision 11)  
- [x] `KeyEvent` + `App::dispatch` in `xei-core` (TUI via thin crossterm adapter)  
- [x] `suisei-engine` workspace member (cdylib + rlib)  
- [ ] Performance budgets measured (later S6)  

### S1 — Skeleton host  ← **mostly done**

- [x] Pick face: **Swift**  
- [x] `suisei-engine` cdylib: `dispatch_key` + chrome `FrameDiff`  
- [x] SwiftUI xei-like chrome + C ABI bridge  
- [x] Viewport sync (multi-line scroll after Enter)  
- [x] Block caret + wheel scroll + ⌘O open file  
- [ ] Display-link tick; save (⌘S / :w); quit path polish  
- [ ] Metal / styled `PaintLine` blit (→ S2)  

### S2 — Editor v1

- Open file, type, scroll, highlight spans from core  
- Cursor + selection  
- Meet keystroke budget on mid-size files  

### S3 — Input parity

- Full keymap parity tests vs xei  
- Which-key, operators, multi-cursor, leader  

### S4 — Chrome + glass

- Tabs, explorer, status, palette  
- macOS vibrancy / liquid glass polish  
- Settings: `document_layout` toggle UI (still only Tabs wired)  

### S5 — Depth

- Terminal PTY panel, git workbench, DAP panel (reuse core state)  
- ext-host attach  
- Webview container for extensions  

### S5.5 — Infinite canvas (optional layout)

- `layout_canvas` + camera pan/zoom  
- Doc cards → same `editor_frame` pipeline when visible  
- Persist card positions per workspace  
- Settings toggle live switch Tabs ↔ Canvas  
- Overview / fit-all; snap clusters optional  

### S6 — Beat VS Code (measure)

- Startup, RSS, typing latency dashboards  
- Profile IPC; collapse hot paths into Rust-only  
- Canvas: only visible cards compose; overview mode budget  

---

## 10. Risks & mitigations

| Risk | Mitigation |
|------|------------|
| Face editor becomes second source of truth | FrameDiff-only; no local buffer (Swift or Svelte) |
| Face boundary becomes Electron-slow | Damage-only; prefer FFI in-proc (Swift) or binary IPC (Tauri) |
| Keymap drift | Shared dispatch + CI golden tests |
| “Full VS Code compat” scope explosion | Stage host; webview last; track % of top-N extensions |
| Glass looks toy-like | Prefer Swift materials (A); if B, system density &gt; blur spam |
| Swift↔Rust FFI friction | Thin C ABI surface; only FrameDiff + input; UniFFI optional |
| Two faces diverge later | One FrameDiff schema crate / golden frame fixtures |

---

## 11. Decisions (locked unless revisited)

1. **Rust is the engine (Core · Compositor · Bridge). Face is not the engine.**  
2. **UI 전체를 Rust로 쓰지 않는다** — face는 Swift(권장) 또는 Tauri+Svelte.  
3. **One keymap / one App.**  
4. **macOS first.**  
5. **Old suisei code is not migrated — greenfield.**  
6. **ext-host is shared with xei**, not forked.  
7. **No full-document DOM/SwiftUI TextEditor as the primary buffer surface** — only blit of `PaintLine`.  
8. **Document layout is dual-mode:** default **Tabs**; optional **Infinite canvas**. Core buffers unchanged; Compositor owns placement.  
9. **Canvas is Suisei-only** — xei TUI stays tab/split chrome forever.  
10. **FrameDiff is the only face input** — renderer stack is swappable.  
11. **macOS v1 face = Swift** (SwiftUI chrome + Metal/Core Text editor). Plan B (Tauri+Svelte) deferred unless revisited.

---

## 12. Next concrete step

**Done:** design lock · `App::dispatch` · `suisei-engine` · Swift hello scaffold.

**Next:**

1. `./scripts/run-suisei-app.sh` — verify `i` / type / Esc updates mode chrome from Core.  
2. S1 polish: open file, proper key matrix, display-link tick.  
3. S2: Metal (or Core Text) blit of `PaintLine` damage — still no face buffer SoT.

Do **not** reintroduce a second keymap in Swift.
