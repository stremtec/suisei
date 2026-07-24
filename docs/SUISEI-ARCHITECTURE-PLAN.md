# Suisei — Architecture & UX Plan

작성 2026-07-21. 대상: `suisei-core` / `suisei-engine` / `suisei-app` (branch `v2`).
선행 문서: `SUISEI-CORE-DESIGN.md`(코어 재작성 청사진), `SUISEI-TODO.md`(미해결 항목).

이 문서는 **6개 워크스트림**의 설계안이다. 각 항목은 *동기 → 아키텍처 → 인터페이스 →
단계별 구현 → 리스크 → 완료 정의(DoD)* 순으로 기술한다. 코드는 아직 없다; 이 문서가
합의되면 착수한다.

> **Implementation note, 2026-07-23:** this remains a forward plan. The
> current app still owns `Engine/App` in the GUI process; no daemon, IPC or
> docking tree exists. Atomic file save is no longer a planned fix:
> `suisei-core::fs_atomic::atomic_write_file` is in use now. See
> `SUISEI-CURRENT-STATE.md` for execution order.

| # | 워크스트림 | 성격 | 선행 조건 |
|---|---|---|---|
| 1 | Daemon 분리 (`suisei-daemon`) | 신규 프로세스 | 없음 |
| 2 | 패널 Docking / Stand-alone | 신규 기능 | 3 (레이아웃 모델) |
| 3 | 패널 리사이즈 안정화 + 전이 효과 | 버그 + 폴리시 | 없음 |
| 4 | Settings UI 개편 | 폴리시 | 없음 |
| 5 | Dark 기본 테마 색 재조정 | 폴리시 | 없음 |
| 6 | 메뉴 바 역할 정리 | 정보 구조 | 2 (Window 메뉴) |

권장 착수 순서: **3 → 5 → 6 → 1 → 4 → 2**.
근거는 §7 참조 (비용 대비 체감, 그리고 2번이 가장 크다).

---

## 1. Daemon 분리 — `suisei-daemon`

### 1.1 동기

현재 편집 상태(버퍼 내용, undo 스택, 커서, 세션)는 **GUI 프로세스의 힙에만** 존재한다.
`suisei-app`이 크래시하면 마지막 `save_file()` 이후의 편집은 전부 소실된다. 이번 세션에서
실제로 크래시를 겪었고(스택 오버플로), 그때 열려 있던 미저장 버퍼는 복구 수단이 없었다.

목표: **GUI 크래시가 데이터 손실로 이어지지 않는다.** GUI는 재시작 후 데몬으로부터
편집 중이던 상태를 그대로 돌려받는다.

### 1.2 아키텍처 — 3-tier

```
┌──────────────────┐        ┌────────────────────┐        ┌──────────────┐
│  suisei-app      │◄─IPC──►│  suisei-daemon     │◄─fs───►│  disk        │
│  (SwiftUI GUI)   │        │  (Rust, headless)  │        │  WAL + files │
│  ephemeral       │        │  durable           │        │              │
└──────────────────┘        └────────────────────┘        └──────────────┘
         │                            │
         │                    ┌───────▼────────┐
         └── crash ──────────►│  MenuBarExtra  │  상태 표시 · 복구 · Quit
                              └────────────────┘
```

**소유권 이전이 핵심이다.** 현재 `App`(코어 상태 기계)은 GUI 프로세스 안에 산다. 이를
데몬으로 옮기고 GUI는 *뷰 + 입력*만 담당하는 씬 클라이언트가 된다. 이는
`SUISEI-CORE-DESIGN.md`의 "an edit never waits for derived state" 원칙과 정합한다 —
이미 `Engine`이 `recompose()` 경계로 상태와 렌더를 분리해 두었으므로, 그 경계를
프로세스 경계로 승격하는 작업이다.

**단계적 이행**(빅뱅 금지):

- **Phase D0 — Shadow journal.** 데몬은 아직 상태를 소유하지 않는다. GUI가 편집
  델타를 데몬에 *복제 전송*하고 데몬은 WAL에만 적는다. 크래시 시 데몬이 복구본을
  제공. GUI 경로는 그대로이므로 회귀 위험이 가장 낮다.
- **Phase D1 — Authoritative state.** `App`이 데몬으로 이주. GUI는 스냅샷을 구독하고
  키/포인터 이벤트를 전송. FFI(`suisei_engine_*`)는 IPC RPC로 1:1 대응 이관.
- **Phase D2 — Multi-client.** 하나의 데몬에 여러 GUI 창/프로세스(§2 stand-alone
  패널 포함)가 붙는다.

### 1.3 IPC 선택

| 후보 | 장점 | 단점 | 판정 |
|---|---|---|---|
| **Unix domain socket + length-prefixed frame** | 의존성 0, Rust/Swift 양쪽 표준 라이브러리, 디버깅 용이(`nc`) | 프레이밍/재연결 직접 구현 | **채택** |
| XPC | launchd 통합, 권한 모델 | Rust 바인딩 부재, Obj-C 브리지 필요 | 기각 |
| gRPC / Cap'n Proto | 스키마 진화 | 빌드 의존성 급증, 이 프로젝트의 no-serde 기조와 충돌 | 기각 |

소켓 경로: `$XDG_RUNTIME_DIR` 부재 시 `~/Library/Application Support/Suisei/daemon.sock`.
프레이밍: `u32 len (LE) || u16 opcode || payload`. 페이로드는 기존 FFI 구조체와 동일한
**고정 오프셋 바이너리 레이아웃**을 재사용한다 — 이 프로젝트는 이미 Swift 쪽에
하드코딩 오프셋 디코더를 갖고 있고(`SuiseiEditorLineC`), serde를 쓰지 않는다.

> ⚠️ `SUISEI-TODO.md`의 함정 재확인: **필드 추가 시 양쪽 디코더의 오프셋을 모두
> 옮겨야 한다.** IPC로 넘어가면 이 위험이 프로세스 경계를 건너므로, opcode마다
> `u16 version`을 넣고 불일치 시 **거부**한다(조용한 오독 금지).

### 1.4 지속성 — WAL + atomic rename

```
~/Library/Application Support/Suisei/
├── daemon.sock
├── journal/
│   ├── <buffer-uuid>.wal        # append-only edit deltas
│   └── <buffer-uuid>.meta       # path, mtime, encoding, cursor
└── recovery/
    └── <buffer-uuid>.snapshot   # periodic full text
```

- **WAL 레코드**: `seq(u64) || op(u8) || byte_range || utf8 payload`. `Edit`/`Delta`
  타입은 `SUISEI-TODO.md` §9(rope 재작성)가 도입 예정인 그것과 **동일한 타입을 공유**한다.
  즉 이 작업은 rope 마이그레이션의 선행 투자다.
- **fsync 정책**: 매 키스트로크 fsync는 불가(§ 지연 예산). `write()`는 즉시,
  `fsync()`는 **250 ms 디바운스 또는 4 KiB 누적** 시. 크래시 시 최대 손실 250 ms.
- **스냅샷 압축**: WAL이 스냅샷 대비 2× 크기를 넘으면 새 스냅샷 기록 후 WAL 절단.
- **실제 파일 저장은 이미 원자적이다**: `app.rs:save_file()` now uses
  `fs_atomic::atomic_write_file` (`write(tmp) → fsync(tmp) → rename(tmp, path)`).
  D0/D1 must retain this invariant and add recovery for edits that have not
  reached an explicit file save.

### 1.5 MenuBarExtra (메뉴 바 우측 상태 항목)

SwiftUI `MenuBarExtra`(macOS 13+) 사용. 데몬은 헤드리스 Rust이므로 UI를 직접 그릴 수
없다 → **`SuiseiDaemonAgent`라는 별도 경량 Swift 앱**(`LSUIElement=true`, Dock 미표시)이
상태 항목을 소유하고 데몬 소켓을 구독한다.

표시 내용:
- 아이콘: 데몬 상태를 심볼로 — 정상 `circle.fill`(dim), 미저장 편집 존재 시 accent,
  복구 대기 시 `exclamationmark.triangle.fill`.
- 메뉴: 열린 버퍼 수 · 미저장 수 · 마지막 저널 flush 시각 · **Recover…** ·
  **Open Suisei** · **Quit Daemon**(미저장 존재 시 확인 다이얼로그).

기동: `launchd` LaunchAgent (`~/Library/LaunchAgents/com.stremtec.suisei.daemon.plist`),
`KeepAlive.SuccessfulExit=false`(크래시 시 재기동, 정상 종료 시 유지 안 함),
`RunAtLoad=true`. 최초 실행 시 GUI가 plist를 설치하고 사용자에게 고지한다.

### 1.6 리스크

| 리스크 | 완화 |
|---|---|
| 키 입력 지연 증가 (IPC 왕복) | 로컬 낙관적 적용 후 데몬 확인 — 이미 `typeFast` 패턴이 존재. **`keystroke_latency.rs` 벤치를 D1 게이트로 사용**(현재 1.21 ms; 회귀 임계 3 ms) |
| 데몬 자체 크래시 | 데몬은 GUI보다 훨씬 작은 표면. `panic = "abort"` 대신 **catch_unwind로 세션 격리**, WAL은 이미 디스크에 |
| 좀비 데몬 | 소켓에 heartbeat, 클라이언트 0 상태 30분 지속 시 자진 종료(단 미저장 버퍼가 있으면 유지) |
| 버전 불일치 (구 GUI + 신 데몬) | 핸드셰이크에 프로토콜 버전, 불일치 시 GUI가 데몬 재기동 요구 |

### 1.7 DoD

- `kill -9 $(pgrep Suisei)` 후 재실행 → 미저장 편집 내용·커서·스크롤 복원.
- 저장 중 `kill -9` → 원본 파일 무손상(원자적 rename 검증).
- 메뉴 바 항목이 미저장 상태를 실시간 반영.
- 키스트로크 지연 벤치 회귀 3 ms 미만.

---

## 2. 패널 Docking / Stand-alone

### 2.1 동기

현재 레이아웃은 **하드코딩된 3열 구조**다(`sidebarColumn` / `editorCard` /
`inspectorColumn`). 이번 세션에서 인스펙터를 열 밖으로 승격하는 데만 여러 라운드가
들었다 — 위치가 뷰 트리에 박혀 있기 때문이다. 사용자가 패널을 재배치하거나 별도 창으로
떼어낼 수 없다.

목표: 패널이 **런타임 배치 가능한 1급 객체**가 된다.

### 2.2 레이아웃 모델 — 트리

Xcode·VS Code·JetBrains 공통 모델을 채택: **중첩 스플릿 트리 + 탭 그룹**.

```swift
indirect enum LayoutNode {
    case split(axis: Axis, ratio: Double, LayoutNode, LayoutNode)
    case tabGroup(id: GroupID, panels: [PanelID], selected: Int)
}

struct PanelDescriptor {          // 패널의 "정체성" — 위치와 무관
    let id: PanelID               // .project, .scm, .find, .issues,
                                  // .breakpoints, .outline, .file,
                                  // .quickHelp, .terminal, .editor
    let title: String
    let systemImage: String
    let allowedDocks: DockSet     // 어디에 붙을 수 있는가
    let minSize: CGSize
    var placement: Placement      // .docked(GroupID) | .standalone(WindowID) | .hidden
}
```

**Dock 영역**: `.leading` `.trailing` `.bottom` `.center`(에디터 전용).
현재 §"Left = 어디로 갈까 / Right = 이게 뭔가" 규칙은 **기본 배치**로 유지하되
`allowedDocks`로 강제하지 않는다 — 규칙은 권고지 감옥이 아니다.

레이아웃은 `~/Library/Application Support/Suisei/layout.json`에 직렬화(단, no-serde
기조에 따라 코어가 아닌 **Swift 측에서** `Codable`로 저장 — 레이아웃은 순수 프레젠테이션
관심사이므로 코어에 둘 이유가 없다).

### 2.3 드래그 앤 드롭 인터랙션

1. **드래그 개시**: 패널 헤더/탭에서 `NSItemProvider`로 `PanelID` 전달.
2. **Drop zone 하이라이트**: 드래그 중 각 dock 영역에 반투명 오버레이 + 삽입 위치
   프리뷰. 이번 세션에서 만든 `Metaball`/`TravellingPill`의 디자인 언어를 재사용 —
   drop zone 강조는 accent 채움 + Liquid Glass.
3. **Tear-off**: 창 밖에 드롭 → 새 `NSWindow` 생성, `placement = .standalone`.
4. **Re-dock**: stand-alone 창을 본 창의 dock 영역으로 드래그 → 트리에 재삽입.

### 2.4 Stand-alone 창

- `NSWindow` 스타일: `.titled, .closable, .resizable, .fullSizeContentView`.
  타이틀바는 현재 커스텀 크롬과 동일 언어.
- **상태 공유**: 데몬(§1) 완료 후에는 자연스럽다 — 각 창이 동일 데몬의 클라이언트.
  데몬 이전에는 `EngineBridge`를 `@EnvironmentObject`로 공유하고 창별
  `SceneStorage`로 뷰 상태만 분리.
- **닫기 정책**: stand-alone 창을 닫으면 패널은 `.hidden`이 아니라 **원래 dock으로
  복귀**(사용자가 명시적으로 숨기지 않는 한). 사라지는 패널은 학습 불가.

### 2.5 리스크

| 리스크 | 완화 |
|---|---|
| 임의 배치가 §디자인 규칙(좌=이동/우=정보)을 무너뜨림 | 기본 레이아웃은 규칙대로. "Reset Layout" 상시 제공 |
| 레이아웃 상태 폭발 → 재현 불가 버그 | `layout.json` 스키마 버전 + 손상 시 기본값 폴백 |
| stand-alone 창의 포커스/키 라우팅 | 이번 세션의 터미널 포커스 버그 교훈: `focused = true`는 컨테이너를 겨냥하면 엉뚱한 필드로 샌다. 창마다 명시적 first-responder 정책 필수 |

### 2.6 DoD

- 임의 패널을 좌/우/하단 dock 간 이동 가능.
- Tear-off → 독립 창 → re-dock 왕복 무손실.
- 재시작 후 레이아웃 복원.
- "Reset Layout"이 항상 정상 상태로 복구.

---

## 3. 패널 리사이즈 — 떨림 제거 + 전이 효과

### 3.1 근본 원인 (확정)

리사이즈 그립이 **자신이 조절하는 경계 위에 놓여 있다.** `DragGesture`를 `.local`
좌표 공간으로 쓰면 다음 되먹임 루프가 성립한다:

```
손가락 Δ 이동 → 패널 폭 +Δ → 그립도 함께 +Δ 이동
              → 그립 로컬 좌표계 원점이 +Δ → translation이 −Δ 보정
              → 패널 폭 −Δ → 진동
```

`SidebarResizeStrip`만 조용했던 이유는 그것만 `.global`이었기 때문이다.

**적용된 수정**: `OutlineResizeStrip`, `TerminalResizeGrip` 모두
`coordinateSpace: .global`로 변경. (이번 세션에 반영, 사용자 확인 대기)

### 3.2 잔여 안정화 항목

- **정수 스냅 일관성**: 세 그립이 `.rounded()` 타이밍이 제각각이다. 공통
  `ResizeController`로 통일 — `base + Δ`를 클램프 후 **1 pt 단위 양자화**, 변화 없으면
  `size` 쓰기 자체를 생략(불필요한 뷰 무효화 제거).
- **Transaction 격리**: 드래그 중에는 `transaction.disablesAnimations = true`를
  일관 적용. 현재 `SidebarResizeStrip`만 하고 있다.
- **리사이즈 중 코어 통지 억제**: `windowLiveResizing` 플래그가 이미 있으나 그립
  드래그에는 미적용. 드래그 중 `recompose()` 호출을 억제하고 `onEnded`에서 1회
  `settleEditorResize()`.

### 3.3 전이 효과 — "영향받는 패널"의 시각적 응답

사용자 요구: 리사이즈 시 영향받는 패널에 블러/애니메이션.

**설계 원칙**: 효과는 *장식이 아니라 정보*여야 한다. 리사이즈 중 콘텐츠 재레이아웃은
시각적 소음이므로, 그 소음을 **의도적으로 흐리는** 것이 목적이다.

```
드래그 시작 →  영향 패널: content blur 0 → 3 pt (0.12 s ease-out)
                          + saturation 1.0 → 0.92
드래그 중   →  블러 유지, 레이아웃만 갱신 (텍스트 리플로우 소음 은폐)
드래그 종료 →  blur 3 → 0 (0.2 s ease-out), 동시에 최종 레이아웃 확정
```

- 대상: 폭이 변하는 이웃 패널의 **콘텐츠만**(크롬·헤더·테두리는 선명 유지).
- **에디터 캔버스는 예외**: blur 대신 opacity 1 → 0.85.
- Blueprint Measure Mode 실험은 **철회** (2026-07-22) — 모눈/치수 오버레이가
  제품 톤과 맞지 않아 제거. 그립은 기존 `SidebarResizeStrip` /
  `OutlineResizeStrip` / `TerminalResizeGrip` 로 복귀.

### 3.4 DoD

- 세 그립 모두 드래그 중 진동 없음 (60 fps 캡처로 프레임별 폭 단조성 확인).
- 리사이즈 중 CPU 사용률이 현재 대비 증가하지 않음.

---

## 4. Settings UI 개편

### 4.1 현황 문제

- 탭 인덱스가 **정수 상수**로 분기(`case 0: aboutSections … default: generalSections`) —
  항목 추가 시 실수하기 쉽고 의미가 코드에 없다.
- 카테고리 구성이 macOS 관례와 불일치: About이 첫 탭. 시스템 설정 앱은
  **General → 기능별 → About**(또는 About을 앱 메뉴로) 순.
- 검색 없음. 항목이 늘면 탐색 불가.
- 설정 항목이 코어(`settings.rs`)와 Swift 양쪽에 분산.

### 4.2 설계

```swift
enum SettingsPane: String, CaseIterable, Identifiable {
    case general, appearance, editor, languages, terminal,
         keybindings, extensions, advanced
}
```

- **`Settings` scene + `TabView(.sidebarAdaptable)`** 채택 (macOS 15+ 관례).
  현재 커스텀 창 대신 SwiftUI `Settings {}` scene을 쓰면 ⌘, 처리·창 복원·
  위치 기억을 시스템이 담당한다.
- **검색 필드**: 모든 pane의 항목 레이블을 인덱싱, 매칭 항목으로 점프 + 하이라이트.
- **`Form(.grouped)` 통일**: 현재 §"Xcode 인스펙터 폼"에서 확립한 레이블 우측 정렬
  규칙을 설정에도 적용하지 **않는다** — 설정 창은 시스템 설정 관례(좌측 정렬 레이블 +
  우측 컨트롤)를 따른다. 두 규칙이 다른 것은 의도적이다.
- **Keybindings pane 신설**: 현재 키맵은 코어 하드코딩. 최소한 *조회*라도 제공
  (편집은 후속).
- **Appearance pane**: 테마 선택 + §5의 새 다크 팔레트 미리보기 + 폰트/행간.

### 4.3 DoD

- ⌘, 로 열리고 시스템이 창 위치를 기억.
- 8개 pane 전부 검색 가능.
- 정수 인덱스 분기 소멸.

---

## 5. Dark 기본 테마 색 재조정

### 5.1 근거 데이터 (실측)

Apple 공식 문서는 웹 페치가 본문을 반환하지 않아, **AppKit에서 직접 측정**했다
(`NSAppearance(.darkAqua).performAsCurrentDrawingAppearance`, sRGB 변환):

| 토큰 | macOS 26 Dark | macOS 26 Light |
|---|---|---|
| `systemBlue` | `rgb(0, 145, 255)` | `rgb(0, 136, 255)` |
| `systemGreen` | `rgb(48, 209, 88)` | `rgb(52, 199, 89)` |
| `systemOrange` | `rgb(255, 146, 48)` | `rgb(255, 141, 40)` |
| `systemRed` | `rgb(255, 66, 69)` | `rgb(255, 56, 60)` |
| `systemPink` | `rgb(255, 55, 95)` | `rgb(255, 45, 85)` |
| `systemPurple` | `rgb(219, 52, 242)` | `rgb(203, 48, 224)` |
| `systemYellow` | `rgb(255, 214, 0)` | `rgb(255, 204, 0)` |
| `systemTeal` | `rgb(0, 210, 224)` | `rgb(0, 195, 208)` |
| `systemIndigo` | `rgb(109, 124, 255)` | `rgb(97, 85, 245)` |
| `systemGray` | `rgb(152, 152, 157)` | `rgb(142, 142, 147)` |
| `windowBackground` | `rgb(30, 30, 30)` | `rgb(255, 255, 255)` |
| `underPageBackground` | `rgb(40, 40, 40)` | `rgb(150,150,150) α.90` |
| `separator` | `white α 0.10` | `black α 0.10` |
| `secondaryLabel` | `white α 0.55` | `black α 0.50` |
| `tertiaryLabel` | `white α 0.25` | `black α 0.26` |

> **중요 발견**: 우리 `DARK` 테마의 `mode_insert`/`completion_selected` 등은
> `rgb(10, 132, 255)`인데, 이는 **iOS**의 dark systemBlue다. macOS 26은
> `rgb(0, 145, 255)`. 즉 현재 값은 플랫폼이 틀렸다.

### 5.2 문제 진단

현재 `DARK`(`theme.rs:154`):

| 항목 | 현재 | 진단 |
|---|---|---|
| `editor_bg` `rgb(41,42,48)` | 시스템 `windowBackground`(30,30,30)보다 **밝고 푸르다**. 창 크롬과 톤이 어긋난다 |
| `bg` = `editor_bg` | 배경 레이어링 부재 — Apple의 dark mode는 **elevation에 따라 밝기를 올린다**. 현재는 평면 |
| `border` `rgb(58,59,66)` | 불투명 회색. 시스템은 `white α 0.10`(합성 방식) — 배경이 달라지면 자동 적응 |
| `accent` 계열 `rgb(10,132,255)` | iOS 값 (§5.1) |
| `selection_bg` `rgb(78,90,112)` | 시스템 `selectedTextBackground`(63,99,139)보다 채도 낮고 탁함 |

### 5.3 개편 방향

**(a) Elevation 계층 도입.** 단일 `bg` 대신 3단계:

```
L0 shell      rgb(24, 24, 26)   ← 창 바닥, 패널 사이 채널
L1 surface    rgb(30, 30, 32)   ← 에디터/패널 본체 (= windowBackground 정렬)
L2 raised     rgb(40, 40, 43)   ← 팝오버, 자동완성, 터미널 밴드 (= underPageBackground)
```

이는 이번 세션에 이미 도입한 `shellBase` / `editorBg` / `terminalDockFill`
3층 구조와 정확히 대응한다 — **코드는 이미 이 모델을 요구하고 있었고 테마가 못 따라온
상태**였다.

**(b) 구분선을 알파 합성으로.** `border: Color::Rgb(58,59,66)` →
`Color::Rgba(255,255,255,26)` (α 0.10). `Color` enum에 알파 변형 추가 필요.

**(c) 시스템 accent 정렬.** `rgb(0, 145, 255)`.

**(d) 대비 검증.** WCAG 2.1 기준 본문 텍스트 4.5:1, 큰 텍스트/UI 3:1.
현재 `fg(223,223,224)` on `editor_bg(41,42,48)` = 약 11.6:1 (충분).
**검증이 필요한 것은 신택스 컬러다** — `comment rgb(108,121,134)` on 신규
L1(30,30,32) = 약 5.4:1(통과), `line_no rgb(116,116,122)` = 약 5.2:1(통과).
전 토큰에 대해 자동 검사를 CI에 추가한다.

**(e) 검증 도구.** `suisei-core/tests/theme_contrast.rs` 신설 — 모든 테마의 모든
전경/배경 조합에 대해 상대 휘도 대비를 계산하고 임계 미달 시 실패.

### 5.4 DoD

- Dark 테마가 시스템 크롬(타이틀바·메뉴·시트)과 톤 연속.
- 대비 테스트 전 항목 통과.
- Light 테마 회귀 없음.

---

## 6. 메뉴 바 역할 정리

### 6.1 현황 문제

`SuiseiApp.swift`의 커맨드 구성이 **역할 경계가 흐리다**:

- View 메뉴(`CommandGroup(after: .sidebar)`)에 패널 토글과 **네비게이터 모드 전환**이
  섞여 있다. 전자는 가시성, 후자는 콘텐츠 선택 — 다른 범주다.
- `CommandMenu("Editor")`는 macOS 표준 메뉴가 아니다. 내용이 무엇이냐에 따라
  Format 또는 View로 흡수되어야 한다.
- Window 메뉴가 기본값 그대로 — §2 stand-alone 창이 들어오면 반드시 확장 필요.
- ⌃F(File Explorer) 같은 **컨트롤 키 단축키**가 메뉴에 있다. macOS 관례상 메뉴
  단축키는 ⌘ 기반이어야 한다(⌃는 텍스트 편집 emacs 바인딩 영역).

### 6.2 표준 배치 (Apple HIG 메뉴 관례)

| 메뉴 | 담당 | Suisei 항목 |
|---|---|---|
| **Suisei** | 앱 전역 | About · Settings ⌘, · Services · Hide · Quit · **Daemon Status**(§1) |
| **File** | 문서 생명주기 | New Tab ⌘T · New Window ⌘N · Open ⌘O · Open Recent ▸ · Close Tab ⌘W · Save ⌘S · Save As ⇧⌘S · Revert |
| **Edit** | 텍스트 변형 | Undo/Redo · Cut/Copy/Paste · Select All · **Find ▸**(Find ⌘F, Find in Project ⇧⌘F, Replace ⌥⌘F) |
| **View** | **가시성만** | Show/Hide Navigator ⌘0 · Inspector ⌥⌘0 · Debug Area ⇧⌘Y · Minimap · Enter Full Screen |
| **Navigate** | **위치 이동** | Go to Line ⌘L · Go to File ⇧⌘O · Back/Forward ⌃⌘← → · Jump to Definition ⌃⌘J · **Navigator 모드 ⌘1‥⌘5**(View에서 이동) |
| **Editor** → **Format** | 텍스트 표현 | Indent/Outdent · Comment ⌘/ · Fold ▸ · Font Size ⌘+ ⌘− |
| **Terminal**(신설) | 셸 | New Shell ⌃T · Next/Previous Shell · Clear · Kill |
| **Window** | 창 관리 | Minimize · Zoom · **Tear Off Panel ▸**(§2) · **Merge All Windows** · Reset Layout · 창 목록 |
| **Help** | 문서 | Suisei Help · Keyboard Shortcuts · Release Notes |

**핵심 재배치 3건**:
1. 네비게이터 모드(⌘1‥⌘5) → **View에서 Navigate로**. 모드 전환은 "어디를 볼까"이지
   "무엇을 보이게 할까"가 아니다 (§좌측 레일 설계 규칙과 동일 논리).
2. `Editor` → **Format**으로 개명 (macOS 표준 메뉴명).
3. Terminal 메뉴 신설 — 현재 셸 명령이 여러 메뉴에 흩어져 있다.

### 6.3 DoD

- 모든 메뉴 단축키가 ⌘ 기반(텍스트 편집 emacs 바인딩 제외).
- 각 메뉴 항목이 정확히 한 범주에 속함.
- Window 메뉴가 stand-alone 창을 나열.

---

## 7. 실행 순서와 근거

```
3 (리사이즈)  ──┐
5 (다크 테마) ──┼── 저비용·고체감, 서로 독립. 즉시 착수 가능
6 (메뉴 정리) ──┘

1 (데몬)      ──── 데이터 안전. D0(shadow journal)만으로도 크래시 손실 해결
                   ※ 단, §1.4의 **원자적 저장**은 데몬과 분리해 지금 즉시 수정

4 (설정 UI)   ──── 5의 Appearance pane과 함께 하면 중복 작업 없음

2 (도킹)      ──── 가장 큼. 1(D1)과 3(ResizeController)이 선행되어야 안정적
```

**즉시 처리 권고 (플랜 외 긴급)**: `app.rs:5081`의 비원자적 `fs::write`.
저장 중 크래시가 사용자 원본을 파괴할 수 있으며, 이는 데몬 없이도 3줄로 고쳐진다.

---

## 8. 이 문서가 의존하는 기존 결정

- **레이아웃 문법**: 아일랜드(에디터+터미널)가 플로팅 네비게이터 아래를 지나가고,
  인스펙터는 전체 높이 컬럼. 상태바는 셸 톤. → `SUISEI-TODO.md` "Final layout grammar"
- **디자인 언어**: `Metaball`/`SplitCapsule`/`TravellingPill`, Liquid Glass는
  `GlassEffectContainer` + 사이즈 있는 뷰 구조. → 동 문서 "Sidebars" 절
- **성능 기준선**: 키스트로크 1.21 ms, 증분 파싱 + 오버스캔 400행. → 동 문서 §Traps
- **빌드**: `-O` 4분 / `SUISEI_FAST=1` 27초, 배치 모드 금지. → 동 문서 §Traps
