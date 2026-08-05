# Suisei — 편집 아키텍처 재설계 ("로딩은 길어도, 편집은 최상")

작성 2026-07-31. 대상: `suisei-core` / `suisei-engine` / `suisei-app`.
선행 문서: `SUISEI-CORE-DESIGN.md`(rope 저장 단계 청사진 — 여기서 말하는 P2와 동일),
`SUISEI-ARCHITECTURE-PLAN.md`(UX 워크스트림), `SUISEI-CURRENT-STATE.md`(실행 순서).

> 이 문서는 **편집 코어 자체의 재구성** 설계다. `ARCHITECTURE-PLAN`이 워크벤치 UX를
> 다룬다면, 이 문서는 그 아래 — 키 한 번이 화면에 닿기까지의 경로 — 를 다룬다. 코드는
> P1부터 착수한다. 각 절은 *동기 → 아키텍처 → 인터페이스 → 단계 → 리스크 → DoD*.

---

## 0. 원칙

> **편집 품질 = (핫패스가 얼마나 얇은가) × (필요한 게 얼마나 미리 데워졌는가).**

사용자 요구를 그대로 옮기면 **"로딩은 길어도 편집경험은 최상"**. 이건 취향이 아니라
설계 제약이다: 시작 시점에 **비싼 것을 미리 계산할 예산**이 있고(로딩이 길어도 됨),
대신 **키를 누르는 순간의 경로는 최소이며 뜨거운 자료만 만진다**(편집이 최상).

두 축으로만 뜯는다:

1. **핫/웜/콜드 3-tier 실행 모델** — 모든 작업을 티어로 분류하고 위 티어로 새는 것을 금지.
2. **트랜잭션 문서 코어** — rope + transaction + carets + undo 를 하나의 primitive에서 파생.

나머지 증상(IME 재정렬, undo 과다 되돌림, 분할 렉)은 전부 이 둘의 파생 결과로 사라진다
(§8).

**근거 — 이미 한 서브시스템이 정답대로 산다.** `suisei-core/src/syntax.rs`:
> "opening a file costs one cold parse; edits after that are incremental."
> (6k줄에서 2.75ms → 0.1ms.)

`typeFast`(풀 chrome 우회), `scheduleChromeSettle`(120ms 코얼레스), pull 렌더러
(`EditorCanvasView`가 dirty 밴드만 엔진에서 당김) 도 같은 방향의 흩어진 조각들이다.
**이번 작업은 새 개념의 발명이 아니라, syntax가 이미 따르는 규율을 전 프로그램의 법칙으로
승격시키는 것.**

---

## 1. 3-tier 실행 모델

### 1.1 정의

| 티어 | 트리거 | 예산 | 무엇이 도는가 |
|---|---|---|---|
| **핫** | 키 1회 | **< 1ms, 초과 금지** | rope splice · 트랜잭션으로 커서 매핑 · **포커스 캔버스 밴드만** 리페인트 |
| **웜** | 코얼레스(키에서 분리) | 프레임 예산 내, 논블로킹 | 증분 tree-sitter 재파싱 · LSP `didChange`(디바운스) · chrome/status(settle) · git gutter |
| **콜드** | 부팅 / 프로젝트 오픈 | **길어도 됨** | LSP `spawn+initialize+didOpen` · 워크스페이스 인덱스 · 심볼 인덱스 · 세션 파일 프리파스 · 글리프 아틀라스 · 그래머 로드 |

### 1.2 불변식 (invariants)

- **핫패스는 절대 파일 전체를 만지지 않는다.** `buffer.text()`(전체 join)·전체 재파스·
  전체 토큰 재빌드·180KiB 스냅샷 pull — 전부 핫에서 금지.
- **웜은 핫을 절대 막지 않는다.** 웜 작업은 rope **스냅샷**(불변, 구조 공유) 위에서 돌고,
  결과만 다음 프레임에 반영. 편집은 그동안 새 rope 버전으로 진행.
- **콜드는 편집을 하드 게이트하지 않는다** (§3.3 프로그레시브 readiness).

**"모든 히칭 = 잘못된 티어에서 도는 작업."** 히칭 제거란 곧 작업을 한 티어 아래로 내리는 것.

### 1.3 현재 코드의 어긋남 (고칠 대상)

- 핫패스가 웜/콜드 일을 함:
  - `EngineBridge.refreshEditorPaintOnly()` 가 키마다 180KiB `SuiseiChromeSnapshot`
    pull + coarse `@Published`(chrome/editorLines/editorSplit) → SwiftUI 트리 전체
    무효화. 분할 시 트리가 커져 비용↑ (= 분할 렉).
  - `App::push_undo()` → `Buffer::snapshot()` = `lines.clone()` (문서 전체 clone).
- 콜드가 없어서 웜/핫이 콜드 일을 함:
  - 첫 파일 열 때 콜드 파스, 첫 완성에서 LSP 콜드스타트, 첫 팔레트에서 FS 스캔 —
    전부 편집 중에 처음 발생 → 멈칫.

---

## 2. 트랜잭션 문서 코어 (척추)

### 2.1 단일 편집 primitive `Transaction`

타이핑 / 붙여넣기 / IME / 삭제 / 멀티커서 / LSP 포맷·rename — **모든** 변경을
`(range → replacement)` 집합으로 **원자 적용**하는 단 하나의 통로.

- **자기 자신을 통해 모든 위치를 매핑**한다: 커서·선택·폴드·진단·LSP 좌표. 편집 후
  위치는 "다시 계산"이 아니라 "트랜잭션에 통과"시켜 얻는다.
- **역연산을 자동 생성** → 진짜 undo 델타(사후 스냅샷 diff 폐기). undo 그룹 경계 =
  트랜잭션 경계 + 타이핑 런 코얼레스 정책(공백/개행/커서이동/붙여넣기에서 끊음).
- **커서 권위 단일화.** 현재 `buffer.cursor`(원시) 와 `sel: SelectionSet`(GUI) 이중
  권위 + `gui_insert_text`(sel 기반)/`paste_text_at_cursor`(cursor 기반) 이중 경로가
  IME 재정렬 버그의 근원(§8). 트랜잭션이 유일 통로가 되면 그 버그 클래스가 구조적으로 소멸.

**레일은 이미 있다.** `suisei-core/src/edit.rs`: `Change`(insert/delete/replace +
`inverse()`), `Edit`, `Delta`, 그리고 `Buffer::apply_edit(&Edit) -> Delta`. 편집
경로가 이걸 안 쓰고 `insert_str`/`set_line`/`delete_range` 로 우회할 뿐.

### 2.2 rope 버퍼

`Vec<String>` → rope(ropey 또는 자체 piece-tree).

- 편집 O(log n) (현재 `char_to_byte` O(col) + 라인 tail memmove O(len)).
- **구조 공유 불변 스냅샷**: `clone` = Arc bump. → undo 스냅샷이 공짜가 되고,
  **웜/콜드 작업이 스냅샷을 든 채 핫패스를 안 막고 읽는다** (§1.2 불변식의 물리적 근거).
- 오프셋 기반 인덱싱이라 §2.1 트랜잭션과 그대로 맞물림. `CORE-DESIGN.md`가 "next
  storage phase"로, `buffer.rs`가 "line index replaces this scan"로 예고한 지점.

### 2.3 carets / undo 파생

`Document`는 `sel: SelectionSet` 를 유일 커서 권위로 두고, `buffer.cursor` 는 primary
caret의 파생 뷰로 축소. undo 스택 = 트랜잭션 역연산의 스택(스냅샷 아님).

---

## 3. 부팅 파이프라인 (콜드 티어의 구현)

### 3.1 두 개의 로딩 순간

콜드 워밍업은 성격이 다른 **두 시점**에 나뉜다. 둘 다 같은 스테이지 UI(§3.5)를 쓴다.

1. **앱 부트 (웰컴 화면)** — 프로젝트가 아직 없음. **전역** 워밍업: 폰트/글리프 아틀라스,
   그래머 로드, 세션 복원(마지막 파일들), recents. 짧다. *(현재 구현됨: `WelcomeView`의
   `bootStages` + 상태 텍스트 + reveal.)*
2. **프로젝트 오픈 (웰컴→에디터, 또는 폴더 열기)** — **여기가 긴 로딩**이자 "편집 최상"의
   본무대. **프로젝트별** 워밍업: 파일 인덱스, LSP spawn+initialize, git status, 열린
   파일 파스. *(P3에서 이 전이에 같은 로더를 얹는다.)*

### 3.2 스테이지 표

각 스테이지 = `{ 상태 텍스트, 데우는 캐시, 없애는 히칭 }`.

| 시점 | 스테이지 | 데우는 캐시 | 없애는 히칭 |
|---|---|---|---|
| 부트 | Preparing editor | 글리프 아틀라스 · 셀 메트릭 | 첫 페인트 잰크 |
| 부트 | Loading grammars | tree-sitter 언어 로드 | 첫 하이라이트 그래머 로드 |
| 부트 | Restoring session | 세션 파일 → rope + 라인 인덱스 | 첫 파일 열기 지연 |
| 오픈 | Scanning files | 파일트리 + 퍼지파인드 인덱스 | 첫 팔레트/goto-file 지연 |
| 오픈 | Parsing syntax | 열린 파일 tree-sitter 콜드 파스 | 첫 하이라이트 멈칫 |
| 오픈 | Building symbols | 워크스페이스 심볼 인덱스 | 첫 goto-symbol 지연 |
| 오픈 | Warming language server | spawn + initialize + didOpen, 첫 진단까지 | **첫 완성/hover 콜드스타트(수백 ms)** |
| 오픈 | Reading Git | status / blame / graph 캐시 | 첫 gutter/blame 지연 |

### 3.3 프로그레시브 readiness (하드 게이트 아님)

편집은 `[문서 코어 + 그 파일 syntax]` 만 데워지면 **즉시 시작**. 나머지(전체 심볼, 전
파일 LSP, git graph)는 백그라운드로 계속 데워지며 **준비되는 대로 기능이 켜진다**. 상태
텍스트가 "아직 데우는 중"을 표시.

→ "로딩 길어도"가 *"유저가 전부 기다린다"* 가 아니라 *"필요해지기 전에 끝나 있다"* 가 된다.
최소 블로킹은 작게, 비싼 것은 미리 — 이게 원칙의 정확한 구현.

### 3.4 로딩 문구 (copy) — 확정: **Plain-technical**

문구는 자주 보이므로 **정직 + 구체 = 프로다움**. 정확히 뭘 하는지 말해주는 톤으로
확정(미니 진단 겸용, 동적 카운트로 예쁨과 정보를 동시에). 스테이지 label은 여기 한 곳에서만
정의하고 코어/페이스가 공유하며, 작업이 콜드로 이관돼도 문구는 안정적으로 유지한다.

**앱 부트 (웰컴)** — 현재 구현/배선됨:

| label | 실제 작업 | 상태 |
|---|---|---|
| `Preparing editor` | 글리프 아틀라스 · 셀 메트릭 warm | 실제 (face) |
| `Loading grammars` | tree-sitter 전 언어 grammar/query 사전 빌드 (`suisei_engine_warm_grammars` → 워커 `warm_all`) | **실제 (core)** |
| `Restoring session` | recents/세션 prime | 실제(경량), P3에서 파일 복원까지 확장 |

**프로젝트 오픈** — P3에서 이 전이에 같은 로더를 얹으며 채운다:

| label | 실제 작업 |
|---|---|
| `Scanning N files` | 파일트리 + 퍼지파인드 인덱스 (N = 동적 카운트) |
| `Parsing syntax` | 열린/최근 파일 `prewarm_file` (이미 존재) |
| `Building symbol index` | 워크스페이스 심볼 |
| `Warming <server>` | LSP spawn+initialize+didOpen (`<server>` = rust-analyzer 등) |
| `Reading Git status` | status/blame/graph 캐시 |

> 규칙: **하는 일 없는 label 금지.** 부트 티어는 위 3개가 실제 작업을 하므로 지금 노출하고,
> 프로젝트-오픈 스테이지는 각 워밍업이 실제로 붙는 P3 시점에 노출한다.

### 3.5 인터페이스

- **페이스**: `BootStage { label, run: () async -> Void }` 배열을 로더가 순회하며
  `label` 을 상태 텍스트로 노출, `run` 실행. 라벨 legibility floor(240ms) + 전체
  min-splash floor(650ms)로 깜빡임 방지. 완료 시 액션/recents가 rise-in.
  *(구현됨: `WelcomeView` / `SuiseiApp.WelcomeSceneRoot`.)*
- **코어(P3)**: 스테이지가 코어 워밍업으로 이관되면, 진행 상태를 작은 채널(현
  chrome 스냅샷의 한 필드 또는 전용 이벤트)로 흘리고 페이스가 바인딩. 무거운 작업은
  백그라운드 실행기에서(메모리: "GUI must pump async clients" 의 pump에 얹음), 메인은
  진행률만 수신.

---

## 4. 반응성 재배선 (분할 렉 뿌리 제거)

- **폐기**: 키마다 180KiB pull + coarse `@Published` → 트리 전체 무효화.
- **대체**: 코어가 `version: u64` + 서피스별 dirty 신호. 캔버스는 이미 하듯 버전으로 밴드
  pull(핫). tabs/status/gutter/panels 는 settle 틱에서만, 각각 독립 신호(웜). **각 pane은
  자기 문서 슬라이스만 관찰** → 분할 비용이 트리 크기와 무관해짐.
- 부분적으로 이미 존재(`scheduleChromeSettle`, `typeFast`, pull 렌더러) — 규율로 확장.

---

## 5. Document ⟂ Workbench 분리 (갓 오브젝트 해체)

- `Document { rope, carets, undo, version, tree, diagnostics }` — LSP/git/terminal/
  panel 을 **모른다**.
- `Workspace { documents, lsp, index, git, panels, tabs, split, layout }` —
  Document 를 **관찰**한다.
- 키 입력 = Document 변경 + version bump. Workspace 는 웜티어에서 반응 → 키 하나가 갓
  오브젝트(현 `App`)를 흔들지 않음.

---

## 6. 마이그레이션 (빅뱅 금지, 의존 순서)

| 단계 | 내용 | 사는 것 | 리스크 | 선행 |
|---|---|---|---|---|
| **P1** | 트랜잭션 통일 (현 `Vec<String>` 위): 모든 편집 → `apply_edit`, undo=역연산, 커서=트랜잭션 매핑 | **IME·undo 버그 구조적 해결** | 낮음 | 없음 |
| **P2** | rope 교체 (Buffer 내부만, 오프셋 API 유지 → downstream 무변) | O(log n) + 공짜 스냅샷 | 중 | P1 |
| **P3** | 3-tier 확립 + 부팅 파이프라인에 실제 워밍업 부착 (세션 프리파스·인덱스·LSP·git) | **"편집 최상" 페이오프** | 중 | P2 |
| **P4** | 반응성 재배선 + per-pane 관찰 | 분할 렉 해소 | 중 | P2 |
| **P5** | Document / Workspace 분리 | 유지보수성 | 큼 | P1 |

착수 근거: **P1이 지금 아픈 버그를 즉시 해결하면서 나머지 전부의 전제.** 이후 P2가 성능
바닥, P3가 로딩 페이오프, P4가 분할, P5가 구조 정리.

---

## 7. 단계별 DoD

- **P1**: `insert_str`/`set_line`/`delete_range` 직접 호출이 편집 경로에서 사라지고 모두
  `apply_edit` 경유. undo가 스냅샷 diff를 안 씀. 타이핑 런이 공백/개행/커서이동/붙여넣기
  경계에서 끊김. IME 재정렬·undo 과다 회귀 테스트 통과.
- **P2**: `Buffer` 내부가 rope. `snapshot()` 이 O(1)(Arc). 6k줄 파일 편집 p95 프로파일
  개선치 기록. 기존 테스트 무변 통과.
- **P3**: 프로젝트 오픈이 스테이지 로더를 띄우고, 첫 완성/hover/goto-symbol/gutter 가
  **콜드스타트 없이** 즉시. 프로그레시브 readiness 로 최소 게이트 후 편집 가능.
- **P4**: 분할 상태 타이핑 프로파일에서 트리 무효화 비용 제거. per-pane 관찰 확인.
- **P5**: `Document` 가 LSP/git/panel 심볼을 참조하지 않음(컴파일 경계로 강제).

---

## 8. 이 문서가 해결하는 기존 이슈

- **IME 재정렬** (`안녕하세요 안녕하세요 ` → `안녕하세안녕하세요 요`): 이중 커서 권위 +
  이중 편집 경로의 증상. 현재 `paste_text_at_cursor` 의 `sync_sel_to_cursor()` 반창고로
  막았으나, **P1(트랜잭션 통일)이 근본 해결** — 그때 반창고 회수.
- **undo 과다 되돌림**: `edit_run` 이 커서 이동에서만 끊기고 공백/개행/붙여넣기에서 안
  끊김 + 사후 diff 경계가 입력이 아닌 줄범위 기준. **P1의 코얼레스 정책이 해결.**
- **분할 렉**: 키마다 coarse `@Published` → 트리 전체 무효화. **P4가 해결**(캔버스는 이미
  pull, 나머지를 per-pane/settle로).
- **부팅 프리징(미래)**: 인덱싱/LSP가 붙는 순간 웰컴/오픈이 얼어붙는 문제. **P3 + §3.5
  로더가 선제 차단.**
