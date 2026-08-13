# Suisei 대화형 사용성·기능 감사 보고서

반면 현재 빌드는 일상 편집에 투입하기 전에 반드시 해결해야 할 문제가 있다.
st(selection:)` 대신 페이지 ID를 직접 전달하는 명시적 Button 목록으로 교체 | 네 페이지 클릭·VoiceOver |
| SUI-006 | editor/settings window에 고유 AppKit identifier를 부여하고 편집기 traffic-light 보정을 editor identifier에만 한정 | Settings 열기·테마 전환·traffic lights |
| SUI-007 | LSP diagnostic revision을 추가해 같은 개수의 내용 변경과 clear도 Issues snapshot을 갱신 | 실제 rust-analyzer publish |
| SUI-008 | SCM 행 single-click 선택, double-click 열기, Stage/Unstage context menu 연결 | 실제 행 hit test |
| SUI-009 | 네이티브 TextField의 cut/copy/paste/select-all/undo/redo를 AppKit responder chain으로 분기 | Project Filter·검색 필드 실사용 |
| SUI-010 orkspace`로 변경 | 메뉴 바 확인 |
| SUI-024 | Filter Clear 전에 marked-text field의 focus를 해제하고 다음 runloop에 복귀시켜 ghost composition을 제거 | 실제 한글 조합 중 Clear |
| SUI-025 | Preview immediate-close 시 stale status message를 함께 제거 | Preview 열기/닫기 |
| SUI-026 | 4-pane에서 pane header와 Editor 메뉴의 Split 명령을 미리 disabled, 단일 pane에서 Focus/Close Pane 비활성화 | 1/4-pane 메뉴 확인 |
| SUI-027 | Command Palette에서 `w/wq/q/q!/:bd/gt/gT`와 XLC를 제거하고 macOS 명령명·shortcut으로 교체 | 실제 팔레트 목록 |
| SUI-028 | Command Palette 입력도 네이티브 `TextField`와 full-value query API로 전환 | 실제 한글 조합 |
| SUI-029 | 화면 명령 행을 명시적 SwiftUI `Button`과 accessibility label/hint로 전환 | Accessibility Inspector |
| SUI-030 | titlebar AppKit mouse overlay가 닫기 slot을 먼저 처리해 stable tab ID를 닫도록 수정 | active/inactive/dirty/layout tab |
| SUI-031 | `Project` heading의 leading을 선택 rail의 실제 색상 경계에 맞춰 2pt 안쪽으로 보정 | screenshot 재실측 |
| SUI-032 | Project action 네 개의 glyph size/수직 optical offset을 개별 보정하고 모든 hit box는 28×24pt로 고정 | 1× raster 중심 오차 ≤0.5px |
| SUI-033 | Project·Outline·SCM·Git·Palette list selection을 radius 4 또는 flat tint+leading rule로 통일 | 전체 navigator 시각 비교 |
| SUI-034 | focused pane 전체의 파란 stroke를 제거하고 header rule만 유지 | editor/terminal pane 비교 |
| SUI-035 | 상단 1차 mode 축소, Files/Diff detail화, Status/History master–detail, Navigator 임시 숨김, 중복 footer 제거, 행 클릭 preview, `<800` push navigation, `800–1199` context drawer, `≥1200` 3-pane, 저장되는 master/context resize divider 구현 | 800/1200/1600pt 실측·Git 실제 diff, 폭의 layout별 scope |
| SUI-036 | 탭 바의 14pt 안쪽 fade는 유지하고 chip row 양끝에도 같은 14pt safe inset을 추가해 첫/마지막 탭이 fade 아래에 정지하지 않도록 수정 | 2-tab/overflow strip의 양끝 raster |
| SUI-037 | 일반→그룹→통합의 대칭 3단계와 물리 gesture당 1-step lock을 구현. 그룹 frame cache를 stable ID로 변경하고 explicit smooth/matched geometry 전환 추가 | 실제 `↑↑↓↓` 왕복 통과. 20–280ms 연속 캡처 요청은 Computer Use 내부 대기로 중간 프레임 미포착 |
| SUI-038 | pane header 분할 메뉴를 Left/Right/Above/Below 네 방향으로 확장하고 `Split Left` Core/FFI 경로와 배치 테스트 추가 | 실제 pane 메뉴 네 항목과 Right 분할 통과 |
| SUI-039 | grouped/unified style toggle 시 현재 단계와 다음 gesture를 상태줄에 즉시 반영 | 최신 패키지에서 unified·grouped 복귀 문구 실제 확인 |

추가로 긴급 기능 명세의 pane 헤더 컨트롤을 구현했다. 단일 pane의
jump bar에는 `split ▾`만, 2–4 pane의 각 경로 헤더에는 상시
`split ▾`와 `×`를 표시한다. 분할 메뉴는
`Split Left / Split Right / Split Above / Split Below` 네 방향이며,
4-pane에서는 미리 disabled된다. `×`는 클릭한 pane을 먼저 focus한 뒤
그 pane만 닫는다.

### 1.2 실제 앱 재검증 — 2026-08-01

재시작 허용 뒤 최신 패키지를 실제 Suisei 창에서 다시 시험했다.

- 트래픽 라이트와 Navigator toggle은 반복 재시작·레이아웃 왕복 동안
  같은 y 중심선을 유지했다. 상단 탭, 우측 버튼도 별도 3px offset 없이
  48pt band 안에서 안정적으로 정렬됐다.
- 탭 hover 영역의 `x`는 실제 대상 탭을 닫았다. 가운데 탭 삭제 후 남은
  탭은 새 중심으로 재배치됐고 stable ID가 뒤바뀌지 않았다.
- 탭 바 위 스크롤 2회와 아래 스크롤 2회로
  `일반 → 그룹 → 통합 → 그룹 → 일반`이 정확히 한 단계씩 진행됐다.
  두 문서 그룹의 blue round는 두 chip의 실측 폭에만 나타났고 왕복 후
  사라지지 않았다.
- pane header 분할 메뉴에서 Left/Right/Above/Below 네 방향이 모두
  노출됐고, 분할 상태에서는 각 pane에 split과 close가 상시 보였다.
- `()`와 `""` 자동 완성을 실제 입력으로 확인했다.
- Find는 모든 `target`을 노란색으로 표시하고 1/2 → 2/2 이동 시 현재
  결과의 강조와 caret가 함께 이동했다.
- 외부 한글 `테스트` 붙여넣기와 `한글 선택 복사`의 UTF-8 clipboard
  내용을 확인했다. 이 과정에서 Undo 뒤 SelectionSet이 이전 위치에
  남아 다음 한글 paste가 앞으로 점프하는 추가 결함을 발견해 undo/redo
  시 GUI caret를 복원 cursor에 다시 동기화했다.
- 디스크에서 삭제된 열린 탭은 warning glyph와 취소선을 표시하면서
  버퍼를 유지했다. 실제 앱 tick에 외부 파일 검사가 연결되지 않았던
  누락도 발견해 1초 저주기 poll과 상태 전이 때만 recompose하는 경로를
  추가했다. clean 파일은 두 번의 miss 뒤 닫고 dirty 파일은 Save로
  복구할 수 있게 남긴다.
- 같은 문서를 두 pane에 연 split은 고유 tab chip이 하나뿐이라 기존
  grouped container가 보이지 않았다. 표현할 수 없는 1-document layout
  저장을 거부하고 안내 메시지를 내도록 수정했다.
- 그룹→통합 뒤 상태줄에 이전 grouped 안내가 남던 문제도 발견했다.
  style toggle과 동시에 `Layout unified · scroll down to show member tabs`,
  역방향에서는 `Layout group expanded · scroll up to unify · down to
  unfold`로 갱신하도록 수정하고 최신 패키지에서 두 문구를 확인했다.

Computer Use의 `type_text`는 비ASCII 입력과 OS 입력 소스 전환을 실제
IME 조합 이벤트로 만들지 못했다. 따라서 완성형 한글 paste/copy는 앱에서
확인했지만, 한글 조합 중간의 marked-text 저장은 AppKit/Core 회귀 테스트
통과 상태이며 물리 키보드 최종 확인 항목으로 남긴다. 애니메이션은
15ms 간격으로 최대 8회씩 다시 연속 캡처를 요청했다. 그러나 Computer
Use가 action 뒤 화면 안정화와 캡처를 직렬화해 탭 닫기의 첫 실제 프레임은
1229ms, 일반→그룹 1919ms, 그룹→통합 1278ms, 통합→그룹 1334ms,
그룹→일반 1173ms 뒤에 도착했다. 모두 이미 0.20–0.30초 transaction이
끝난 최종 상태였다. 최종 geometry·대칭 1-step 전이·stable ID는
확인했지만 중간 보간 프레임은 이 도구의 시간 해상도로 판정하지 않았다.

## 2. 심각도 기준

| 등급 | 의미 |
| --- | --- |
| P0 | 데이터 손실, 복구 불가능한 편집 손상 |
| P1 | 핵심 기능이 실패하거나 정상적인 조작을 지속적으로 방해 |
| P2 | 기능은 일부 동작하지만 오해·잘못된 상태·불필요한 우회가 발생 |
| P3 | 시각적 완성도, 라벨, 발견성, 일시적인 상태 표시 문제 |

## 3. 확정 재현 이슈 요약

| ID | 등급 | 영역 | 요약 |
| --- | --- | --- | --- |
| SUI-001 | P0 | 저장/IME | 한국어 조합 중 저장하면 마지막 조합 음절이 디스크에서 누락됨 |
| SUI-002 | P1 | 창 크롬 | 메인 창 트래픽 라이트가 위아래로 계속 튐 |
| SUI-003 | P1 | 찾기/IME | 편집기 Find 입력란이 한국어를 완성형이 아닌 자모로 입력 |
| SUI-004 | P1 | 찾기 | `⌘G`와 다음 화살표가 다음 결과로 이동하지 않음 |
| SUI-005 | P1 | 설정 | Settings 사이드바에서 General·Extensions·Shortcuts를 선택할 수 없음 |
| SUI-006 | P1 | 설정/창 크롬 | Settings의 트래픽 라이트가 사라지고 편집기용 보라색 컨트롤이 좌상단을 덮음 |
| SUI-007 | P1 | 진단/LSP | 실행 중인 rust-analyzer와 확정 구문 오류가 있어도 Issues가 `No issues`를 표시 |
| SUI-008 | P1 | Source Control | 변경 파일 행을 마우스로 클릭해도 선택·열기 동작이 없음 |
| SUI-009 | P1 | 포커스 | 검색·필터 텍스트 필드의 `⌘A`가 입력란 대신 편집기 전체 선택으로 전달됨 |
| SUI-010 | P2 | 프로젝트 검색 | 검색어 지우기 후 이전 검색 결과가 그대로 남음 |
| SUI-011 | P2 | 레이아웃 | 레이아웃을 한 탭으로 병합하면 패널 파일명이 `[No Name]`/`layout 1`로 깨짐 |
| SUI-012 | P2 | 레이아웃 | 레이아웃 저장/복원 기능이 스크롤 제스처로만 노출되어 발견하기 어려움 |
| SUI-013 | P2 | 터미널 | 첫 셸은 활성 파일 폴더, 추가 셸은 `/`에서 시작하여 작업 디렉터리가 불일치 |
| SUI-015 | P2 | 프로젝트 | 새 프로젝트를 열어도 이전 프로젝트의 무관한 탭이 남음 |
| SUI-016 | P2 | 최근 항목 | 최근 프로젝트의 하위 파일을 열면 프로젝트 루트가 `src`로 축소됨 |
| SUI-017 | P2 | 파일 팔레트 | `Go to File`이 Rust 소스만 표시하고 README, Cargo.toml, JSON 등을 누락 |
| SUI-018 | P2 | 설정/브랜딩 | About이 여전히 `xei engine`, `xei-core`, `~/.xei.toml`을 표시 |
| SUI-019 | P2 | Git Workbench | Workbench를 열면 사이드바가 `No repository`/`No local changes`로 모순된 상태를 표시 |
| SUI-020 | P2 | Git Workbench | 변경 수가 동시에 `52`, `55`, 실제 Git 최상위 항목 `4`로 서로 다르게 표시 |
| SUI-021 | P2 | 파일 생성 | New File이 즉시 `Untitled.txt`를 생성하며 이름 입력 포커스를 얻을 수 없음 |
| SUI-022 | P2 | 복구 | 복구 항목을 선택해도 바로 편집기로 이동하지 않고 Welcome으로 돌아감 |
| SUI-023 | P3 | 메뉴 | 메뉴 바에 `View`가 두 번 존재 |
| SUI-024 | P3 | 필터/IME | 한국어 조합 중 Filter 지우기를 누르면 결과는 복원되지만 글자가 포커스 해제 전까지 남음 |
| SUI-025 | P3 | Preview | Pretty Preview를 닫은 뒤에도 상태 표시줄에 Preview 메시지가 남음 |
| SUI-026 | P3 | 다중 패널 | 4분할에서 패널이 실사용이 어려울 정도로 좁아지고, 최대치에서도 Split 명령이 활성 상태 |
| SUI-027 | P2 | Command Palette | macOS 앱 안에 `w`, `wq`, `q`, `q!` 등 TUI/Vim 잔재 명령이 사용자 명령으로 노출됨 |
| SUI-028 | P1 | Command Palette/IME | `한글`을 조합하면 마지막 marked 음절이 보이지 않고 `한`만 표시됨 |
| SUI-029 | P2 | 접근성 | 화면에 보이는 Command Palette 명령 행들이 접근성 트리에 노출되지 않음 |
| SUI-030 | P1 | 탭 바 | 탭 호버 시 나타나는 `x`를 클릭해도 탭이 닫히지 않음 |
| SUI-031 | P3 | Navigator 정렬 | `Project` heading이 선택 pill의 왼쪽보다 1px 돌출되어 보임 |
| SUI-032 | P3 | Navigator 아이콘 | Collapse All glyph가 같은 행의 다른 action보다 작게 보임 |
| SUI-033 | P3 | 전역 선택 UI | 22px row에 6px radius를 사용해 capsule도 사각 highlight도 아닌 애매한 선택 형상 |
| SUI-034 | P3 | Editor pane focus | focused pane 전체를 두르는 파란 테두리가 값싼 selected card처럼 보임 |
| SUI-035 | P2 | Git Workbench 레이아웃 | 고정 28/48/잔여 3열과 9개 동급 탭이 작업 흐름을 끊고 빈 화면을 과도하게 만듦 |
| SUI-036 | P2 | 탭 바 | 좌우 fade 아래에 첫/마지막 탭이 정지해 끝 탭의 글자와 배경이 흐려짐 |
| SUI-037 | P1 | 레이아웃 탭 | 그룹 배경이 이웃 탭과 겹치거나 사라지고 전환 애니메이션이 없으며, 통합에서 한 번 내리면 일반까지 두 단계가 실행됨 |
| SUI-038 | P2 | Editor pane | pane header 분할 메뉴에 위/아래만 있고 좌/우 방향이 없음 |
| SUI-039 | P3 | 레이아웃 상태 | 통합 상태로 바뀐 뒤에도 status가 `Grouped into a layout tab`을 계속 표시 |

정정 기록:

- **SUI-014는 철회했다.** `Ctrl+Shift+T`로 focused editor pane을
  Terminal로 전환할 때 상단에 `Terminal` 탭을 만드는 것은 의도된
  제품 동작이다. Terminal도 tab identity를 가져야 분할 구성을 layout
  tab으로 묶고 복원할 때 문서 pane과 동일한 규칙을 사용할 수 있다.

## 4. 핵심 이슈 상세

### SUI-001 — 한국어 IME 조합 중 저장 데이터 손실

심각도: **P0**

재현 절차:

1. 입력 소스를 한국어로 전환한다.
2. 편집기에서 물리 키 `g k s r m f`를 입력해 `한글`을 조합한다.
3. 마지막 음절이 아직 marked text 상태일 때 `⌘S`를 누른다.
4. 다른 파일로 이동하거나 디스크의 파일 내용을 확인한다.

실제 결과:

- 편집기 화면에는 `한글`이 보였다.
- 저장 성공 상태가 표시됐다.
- 디스크에는 `한`만 저장되고 마지막 `글`이 누락됐다.

기대 결과:

- 저장 전에 marked text를 확정하거나, 조합 중인 문자열까지 포함해 `한글` 전체를 저장해야 한다.

영향:

- 사용자는 저장 성공을 확인하고도 실제 파일에서 문자를 잃는다.
- 한글뿐 아니라 marked text를 사용하는 일본어·중국어 입력에서도 같은 종류의 손실 가능성이 있다.

권장 수정 방향:

- Save 진입 전에 `NSTextInputClient`의 marked text를 commit한다.
- 화면 버퍼, 편집 모델, 저장 스냅샷의 문자열이 동일한지 테스트한다.
- 조합 중 `⌘S`, 자동 저장, 탭 전환, 포커스 이동, 앱 종료를 각각 회귀 테스트한다.

### SUI-002 — 메인 창 트래픽 라이트의 지속적인 수직 진동

심각도: **P1**

재현 절차:

1. Suisei 메인 편집기 창을 연다.
2. 아무 조작 없이 좌상단의 닫기·최소화·확대 버튼을 관찰한다.

실제 결과:

- 연속 캡처에서 버튼 중심이 약 20pt 차이로 위·아래 위치를 반복했다.
- 사용자 조작이 없어도 계속 발생했다.

현재 작업 트리 재검증:

- 상단 컨트롤 행을 잘못 28pt로 축소했던 중간 수정은 폐기했다. 이 값은
  트래픽 라이트뿐 아니라 좌·우 상단 위젯 전체를 `y≈14`로 끌어올렸다.
- 최종 실행 화면에서 트래픽 라이트와 좌측 토글·우측 도구의 공통 중심은
  `y≈23.5~24`로 복구됐다.
- 1× raster에서 Navigator card 경계는 `x=6`, 트래픽 라이트의 유효
  원형 잉크는 `x≈19...74`, 다음 위젯 slot은 `x=88`이다. 안티앨리어싱
  외곽 1px을 제외한 좌·우 여백은 각각 13px다.
- 다른 상단 위젯의 x 좌표는 변경하지 않았다.
- AppKit이 소유한 표준 버튼 frame은 더 이상 한 번도 쓰지 않는다.
  동일한 standard-button cell을 그리는 고정 overlay를 frame view에 한 번
  설치하고 Auto Layout으로 고정했으며, 실제 표준 버튼은 숨겼다.
- 10초 시간차 캡처와 창 zoom→restore 왕복에서 좌표 변화가 없었다.

기대 결과:

- 트래픽 라이트는 창 크기·포커스 변화가 없는 동안 고정되어야 한다.

코드상 추정:

- AppKit의 titlebar 재배치와 20Hz `TrafficLightGuard`의 프레임 재적용이 서로 경쟁하는 것으로 보인다.
- 이는 사용 관찰과 소스 구조를 결합한 추정이며, Instruments나 프레임 로그로 최종 확인이 필요하다.

권장 수정 방향:

- 표준 창 버튼 프레임을 주기적으로 덮어쓰는 구조를 제거한다.
- titlebar accessory와 `fullSizeContentView` 구성을 한 번의 안정된 레이아웃으로 정리한다.
- 주기적 self-healing이 꼭 필요하면 위치 적용 후 AppKit이 다시 바꾸는 원인을 먼저 제거하고, 이벤트 기반으로 제한한다.

### SUI-003 — 편집기 Find의 한국어 IME 미지원

심각도: **P1**

재현 절차:

1. 입력 소스를 한국어로 전환한다.
2. `⌘F`로 편집기 Find를 연다.
3. 물리 키로 영문 `message` 위치의 키를 입력한다.

실제 결과:

- 검색란에 완성형 한글이 아니라 `ㅡㄷㄴㄴㅁㅎ` 형태의 자모가 들어갔다.

교차 검증:

- 같은 실행 세션에서 프로젝트 Filter와 프로젝트 검색 TextField는 `g k s r m f`를 정상적으로 `한글`로 조합했다.
- 편집기 본문도 `한글`을 화면에 조합했다.
- 따라서 시스템 입력 소스 문제가 아니라 커스텀 Find 입력 구현의 문제다.

기대 결과:

- 네이티브 TextField와 동일하게 IME marked text·조합·확정을 지원해야 한다.

### SUI-004 — Find 다음 결과 이동 명령이 서로 다른 검색 상태를 참조

심각도: **P1**

재현 절차:

1. `⌘F`를 누르고 `message`를 검색한다.
2. 표시가 `1 of 2`인지 확인한다.
3. `⌘G`를 누르거나 화면의 Forward 화살표를 클릭한다.

실제 결과:

- 다음 결과로 이동하지 않는다.
- 상태 메시지는 `No search pattern — press / or ? first`로 바뀌며 레거시 검색 상태를 참조한다.
- 반면 아래 화살표 키는 `2 of 2`로 정상 이동한다.
- 2번째 결과에서 Return으로 Find를 닫으면 커서가 다시 1번째 결과로 돌아간다.

기대 결과:

- `⌘G`, Forward 버튼, 아래 화살표가 같은 Find 세션의 next 동작을 실행해야 한다.
- Find를 수락할 때 현재 선택한 결과 위치를 유지해야 한다.

### SUI-030 — 탭 바의 호버 `x` 버튼이 동작하지 않음

심각도: **P1**

재현 절차:

1. 상단 문서 탭 바에서 `README.md` 같은 탭 위에 포인터를 올린다.
2. 탭 오른쪽에 나타나는 원형 `x`를 클릭한다.
3. 호버 상태에서 접근성에 `Help: Close tab`으로 나타난 같은 요소를 다시 클릭한다.

실제 결과:

- 탭이 닫히지 않고 해당 탭이 활성화된 상태로 남는다.
- 탭 수와 열린 문서 목록도 변하지 않는다.
- 같은 요소에 노출된 보조 `Close` 액션을 실행하면 즉시 탭이 닫힌다.
- 테스트 후 Project 트리에서 README를 다시 열어 원래 탭 상태를 복원했다.

기대 결과:

- 호버 `x`를 한 번 클릭하면 해당 탭이 닫혀야 한다.
- 비활성 탭의 `x`를 눌러도 먼저 활성화만 되는 것이 아니라 바로 대상 탭을 닫아야 한다.

코드상 원인 추정:

- `ToolbarTabChip`은 탭 전체를 바깥 `Button(action:)`으로 만들고, 그 label 내부의 `trailingSlot`에 닫기용 `Button`을 다시 중첩한다.
- SwiftUI의 중첩 Button 구조에서 일반 클릭이 내부 close action이 아니라 바깥 탭 활성화 action으로 라우팅되는 증상과 일치한다.
- 보조 `Close` 액션으로는 정상 종료되므로 `engine.closeTabId` 자체보다는 UI hit testing 문제로 판단된다.

권장 수정 방향:

- 닫기 버튼을 탭 활성화 Button의 label 안에 중첩하지 말고, 동일 HStack/ZStack의 별도 sibling hit target으로 분리한다.
- 탭 본문 영역에만 activate gesture를 적용하고 닫기 영역은 명시적으로 이벤트를 소비하게 한다.
- clean/dirty, active/inactive, 일반 탭/레이아웃 탭 각각에 대해 한 번의 클릭으로 정확한 stable ID가 닫히는지 회귀 테스트한다.

### SUI-005 / SUI-006 — Settings 내비게이션 및 창 크롬 손상

심각도: **P1**

재현 절차:

1. 톱니바퀴 또는 `⌘,`로 Settings를 연다.
2. About에서 General, Extensions, Shortcuts를 각각 클릭한다.
3. 잠시 Settings 창의 좌상단을 관찰한다.

실제 결과:

- 모든 클릭이 무시되고 About에 계속 머문다.
- 키보드 위/아래 이동도 선택을 바꾸지 못했다.
- 처음에는 정상적인 빨강·노랑·초록 버튼이 보이지만, 이후 세 버튼이 사라지고 편집기 좌상단의 보라색 캡슐형 컨트롤이 Settings 창 위에 나타난다.
- 숨겨진 접근성 close action으로는 창을 닫을 수 있었다.

기대 결과:

- Sidebar List의 각 행이 마우스·키보드로 선택되어야 한다.
- Settings는 표준 titlebar와 표준 트래픽 라이트를 유지해야 한다.
- 편집기 전용 창 크롬·overlay가 보조 창에 적용되면 안 된다.

추가 관찰:

- About에는 `Native macOS face for the xei engine`, `xei-core 0.1.0`, `~/.xei.toml`이 표시된다.

### SUI-007 — 확정 구문 오류를 Issues가 표시하지 않음

심각도: **P1**

시험 상태:

- `src/main.rs` 끝에 단독 식별자 `한`이 존재했다.
- `cargo check --message-format=short`는 다음 오류로 실패했다.

```text
src/main.rs:8:1: error: expected one of `!` or `::`, found `<eof>`
```

- Suisei가 시작한 `rust-analyzer` 프로세스가 존재했고 작업 디렉터리는 시험 프로젝트 루트였다.

실제 결과:

- 문제 파일을 열고 2초 이상 기다려도 Issues 패널은 `No issues`를 표시했다.

기대 결과:

- 실행 중인 언어 서버가 제공하는 진단이 파일과 Issues 패널에 나타나야 한다.
- 언어 서버 연결 실패라면 `No issues`가 아니라 비연결·오류 상태를 알려야 한다.

### SUI-008 / SUI-019 / SUI-020 — Source Control과 Git Workbench 상태 불일치

심각도: **P1/P2**

Source Control:

- 변경 파일 목록과 선택 강조는 보이지만 각 행을 클릭·더블클릭해도 선택이나 파일 열기가 일어나지 않았다.
- 구현상 행은 시각적인 `HoverRow`이며 실제 Button action이 없는 상태와 일치한다.

Git Workbench:

- Source Control에서 `main`, `55 file(s)`를 표시한 상태로 Workbench를 열었다.
- Workbench가 열린 동안 왼쪽 Source Control은 `No repository`, `No local changes`로 바뀌었다.
- 같은 화면의 Workbench 상단은 브랜치 `main`과 `Local Changes (55)`를 표시했다.
- Changes 헤더 badge는 `52`, 본문은 `Changes 55`였다.
- 실제 `git status --porcelain=v1`은 최상위 기준 4개 항목이었다. 앱은 untracked 디렉터리를 개별 파일로 펼친 것으로 보이나 UI 어디에도 집계 기준이 설명되지 않는다.
- Log는 초기에는 `(loading history…)`에 머물렀고 Log 탭을 클릭한 뒤에야 커밋을 표시했다.

기대 결과:

- 같은 엔진 스냅샷을 사용하는 모든 Source Control UI가 동일한 repository·branch·change 상태를 보여야 한다.
- 변경 수는 집계 기준을 통일하거나 “4 entries / 55 files”처럼 의미를 구분해야 한다.
- 파일 행은 마우스 클릭으로 열리거나 diff를 표시해야 한다.

### SUI-009 — 네이티브 입력란에서 `⌘A`가 편집기로 누출

심각도: **P1**

재현 절차:

1. 프로젝트 검색 입력란에 `message`를 입력한다.
2. 입력란에 포커스가 있는 상태로 `⌘A`를 누른다.
3. Backspace와 추가 문자를 입력한다.

실제 결과:

- 입력란의 텍스트가 선택되지 않는다.
- 상태 표시줄은 편집기 `Selected all`을 표시한다.
- 이후 Backspace와 문자는 검색 입력란에 전달되어, 편집기 선택 상태와 TextField 입력 상태가 동시에 존재한다.
- 같은 현상을 프로젝트 Filter에서도 관찰했다.
- Full Git Workbench가 `Esc back`을 안내하는 상태에서도 Project Filter가
  포커스를 가지면 Esc가 Workbench를 닫지 않았다. 상단 Close 버튼을
  직접 클릭해야 편집기로 돌아왔다.

기대 결과:

- TextField가 포커스된 동안 표준 텍스트 편집 단축키는 TextField가 소비해야 한다.
- 편집기 선택 상태는 바뀌지 않아야 한다.
- Esc처럼 현재 modal/surface를 닫는 명령은 TextField의 cancel 처리 후
  상위 surface까지 전달하는 명확한 responder-chain 정책을 가져야 한다.

### SUI-011 / SUI-012 — 저장 레이아웃 기능의 표시 오류와 발견성

심각도: **P2**

확인된 기능:

- 2분할 상태에서 상단 문서 탭 위로 스크롤하면 현재 분할 구성이 레이아웃 탭으로 접힌다.
- 상태 표시: `Folded into a layout tab · scroll down here to unfold`
- 레이아웃 탭의 컨텍스트 메뉴:
  - `Merge Layout into One Tab`
  - `Unfold Layout`
  - `Close Tab`
  - `Close Other Tabs`
- 병합 후 컨텍스트 메뉴:
  - `Show Layout as Group`
  - `Unfold Layout`
- 탭 위로 아래 방향 스크롤하면 다시 펼쳐진다.

문제:

1. 메뉴 바, 버튼, Help, 온보딩 어디에도 “위로 스크롤해 레이아웃 저장”이 노출되지 않는다.
2. `Merge Layout into One Tab` 실행 직후 실제 내용은 `math.rs`와 `main.rs`인데 패널 제목이 각각 `[No Name]`, `layout 1`로 바뀐다.
3. `Show Layout as Group`을 실행하면 원래 파일명으로 돌아온다.

기대 결과:

- Editor 또는 View 메뉴에 `Save/Fold Current Layout`과 `Unfold Layout` 명령을 제공한다.
- 최초 사용 시 짧은 팁을 표시한다.
- 표시 형식을 바꿔도 각 패널의 문서 제목은 유지한다.

### SUI-013 — 터미널 시작 위치 불일치

심각도: **P2**

확인된 기능:

- Debug Area를 열면 내장 zsh 셸이 나타난다.
- `pwd` 명령 입력과 출력은 정상 동작했다.
- `+`로 셸 세션을 추가할 수 있고 세션을 닫을 수 있다.
- `Ctrl+Shift+T`는 focused pane을 제자리에서 Terminal로 전환한다.
- 이때 상단 `Terminal` 탭이 생성되는 것은 layout tab의 pane identity와
  수명주기를 일관되게 유지하기 위한 의도된 기능이다.

문제:

1. 첫 Debug Area 셸은 활성 파일의 폴더인 `.../src`에서 시작했다.
2. `+`로 만든 두 번째 셸은 `/`에서 시작했다.

기대 결과:

- 기본 작업 디렉터리는 일관되게 프로젝트 루트여야 한다.
- 추가 셸도 같은 프로젝트 컨텍스트를 상속해야 한다.

### SUI-021 — New File 이름 입력 흐름 실패

심각도: **P2**

재현 절차:

1. Project 패널에서 `New File in Folder`를 클릭한다.
2. 생성된 `Untitled.txt` 행을 클릭하고 새 이름을 입력한다.

실제 결과:

- 클릭 즉시 `src/Untitled.txt`가 디스크에 0바이트 파일로 생성됐다.
- 행은 `Value: Untitled.txt`, secondary action `confirm` 상태였지만 일반 TextField로 접근되지 않았다.
- 행을 클릭하고 `audit-note.txt`를 입력하면 이름이 바뀌지 않고 파일이 편집기에 열려 본문으로 입력됐다.
- 기본 이름을 확정하는 secondary action은 동작했다.

기대 결과:

- 파일명을 확정하기 전에는 디스크에 파일을 만들지 않거나, 적어도 생성 즉시 편집 가능한 이름 필드에 포커스해야 한다.
- 클릭·Return·Escape·중복 이름 오류가 예측 가능한 인라인 rename 흐름을 제공해야 한다.

## 5. 기타 확정 이슈

### SUI-010 — 프로젝트 검색 Clear가 결과를 지우지 않음

- `한글` 검색 후 clear 버튼을 누르면 검색 필드는 비지만 `README.md:7`의 이전 결과가 계속 남았다.
- 새 쿼리가 없는 상태임을 나타내는 empty state로 돌아가야 한다.

### SUI-015 — 프로젝트 전환 시 이전 탭 잔존

- 새 시험 프로젝트를 열어도 이전 임시 프로젝트의 `t.txt` 탭이 상단에 남았다.
- 탭이 앱 전역 상태라면 프로젝트 소속을 표시하거나 전환 시 확인 옵션이 필요하다.

### SUI-016 — 최근 항목에서 하위 파일을 열면 루트가 축소됨

- Welcome의 최근 항목 그룹을 클릭하면 먼저 자식 파일이 펼쳐졌다.
- 그 안의 `main.rs`를 열자 Project 트리가 전체 프로젝트가 아니라 `src`만 루트로 표시됐다.
- 프로젝트 폴더를 다시 열어야 Cargo.toml, README, assets가 복원됐다.

### SUI-017 — Go to File 목록 누락

- `⌘P`의 Files 목록은 `main.rs`, `math.rs`만 표시했다.
- 같은 Project 트리에 보이는 `Cargo.toml`, `README.md`, `assets/data.json`, `Cargo.lock`은 검색 대상에서 빠졌다.
- 명령 이름이 `Go to File`이라면 모든 파일을 포함하거나 `Go to Source File`로 의미를 좁혀야 한다.

### SUI-018 — 레거시 xei 브랜딩

- Settings About:
  - `Native macOS face for the xei engine`
  - `xei-core 0.1.0`
  - `~/.xei.toml`
- Suisei의 현재 코어·설정 경로와 일치하도록 교체하거나, 호환성 별칭이라면 설명이 필요하다.

### SUI-022 — 복구 후 편집기로 이동하지 않음

- Recovery 시트에서 시험 프로젝트의 항목을 Recover했다.
- 해당 항목은 목록에서 제거됐지만 시트를 닫은 뒤 Welcome 화면이 나타났다.
- 복구된 문서는 최근 항목을 다시 탐색해야 접근할 수 있었다.

### SUI-023 — 중복 View 메뉴

- 메뉴 바 접근성 트리와 실제 메뉴 구조에 `View`가 연속 두 개 존재한다.
- 첫 번째는 시스템 보기/크기 관련, 두 번째는 File Explorer·Source Control·Find Navigator·Pretty Preview 관련이다.
- 이름을 `Navigator`, `Panels`, `Workspace` 등으로 구분해야 한다.

### SUI-024 — Filter의 한국어 조합 ghost text

- Project Filter에서 `한글`을 조합한 뒤 clear 버튼을 클릭했다.
- 트리 필터링은 즉시 해제되고 clear 버튼도 사라졌지만 TextField에는 `한글`이 계속 보였다.
- 편집기를 클릭해 포커스를 잃은 뒤에야 글자가 사라졌다.

### SUI-025 — Preview 종료 후 상태 메시지 잔존

- Markdown Pretty Preview를 닫으면 2분할 편집기는 정상 복원된다.
- 그러나 상태 표시줄에는 한동안 `Preview · Markdown — Esc close · j/k scroll · r refresh`가 남았다.

### SUI-026 — 4분할 최소 크기와 최대치 피드백

- Right/Below를 조합해 최대 4개 pane을 만들 수 있다.
- 1200px 폭에서 Navigator와 Outline을 함께 열면 일부 pane이 약 150px 폭으로 줄고 탭 이름과 본문이 거의 읽히지 않았다.
- 4개 상태에서도 Split 메뉴가 계속 활성화되며 실행 뒤 상태 표시줄에만 `Max 4 panes`가 나온다.
- 최소 pane 크기, side panel 자동 축소, 최대치에서 메뉴 비활성화 중 하나가 필요하다.

### SUI-027 — Command Palette에 TUI/Vim 잔재가 노출됨

- Navigate → Command Palette를 열면 다음 명령이 macOS 사용자 기능처럼 표시된다.
  - `Save file` / `w`
  - `Save and quit` / `wq`
  - `Quit` / `q`
  - `Force quit` / `q!`
  - `Toggle explorer` / `Ctrl+F`
  - `Toggle side terminal` / `Ctrl+T`
- `Ctrl+F`는 일반적인 Find 기대와 충돌하고, `Force quit`은 문서 손실 가능성이 있는 명령인데 위험도나 확인 절차가 드러나지 않는다.
- 커맨드 팔레트는 Cocoa 메뉴와 동일한 macOS 명령명·단축키를 제공하고, 레거시 TUI 명령은 개발자 모드로 격리하는 편이 안전하다.

### SUI-028 — Command Palette가 마지막 IME 조합 음절을 표시하지 않음

- 한국어 입력 소스에서 물리 키 `g k s r m f`를 입력했다.
- 네이티브 프로젝트 검색은 같은 입력을 `한글`로 표시했지만 Command Palette 검색란에는 `한`만 보였다.
- 마지막 `글` marked text가 검색 UI에 합성되지 않는 것으로 보이며, SUI-001 저장 손실과 같은 조합 문자열 동기화 계열일 가능성이 높다.

### SUI-029 — Command Palette 항목의 접근성 누락

- 화면에는 `Save file`, `Save and quit`, `Quit`, `Force quit` 등 여러 행이 보였다.
- 접근성 트리에는 검색 텍스트와 빈 List만 존재했고 각 명령 행은 버튼·행·텍스트로 노출되지 않았다.
- VoiceOver와 키보드 보조 기술이 명령 이름·선택 상태·위험도를 읽을 수 있도록 각 항목에 명시적인 접근성 representation이 필요하다.

### SUI-031 — Navigator `Project` heading의 왼쪽 optical 돌출

- 사용자가 제공한 354×222 스크린샷에서 선택된 파란 Navigator pill의
  실질적인 색상 경계는 `x=21`에서 시작하지만 `Project` 글자 잉크는
  `x=20`에서 시작했다.
- raster 차이는 1px이고, 소스에서도 title row는 horizontal padding 10,
  선택 pill은 그 안에서 `NavStrip.inset = 2`를 한 번 더 사용한다.
- 수학적 container edge보다 사용자가 실제로 보는 파란 선택 면의 edge가
  정렬 기준이어야 한다.
- `Project` heading의 leading을 1–2pt 안쪽으로 옮기거나 pill의 측정 anchor를
  title row와 공유해야 한다.

### SUI-032 — Collapse All glyph의 optical size 부족

- 같은 스크린샷에서 Project action glyph의 보이는 ink box를 측정했다.
  - New File: 약 12×13px
  - New Folder: 약 16×11px
  - Refresh: 약 10×12px
  - Collapse All: 약 11×11px
- 네 버튼은 모두 `ToolbarPlainIcon(iconSize: 12)`를 사용하지만
  `chevron.left.square`는 사각형 내부 여백 때문에 실제 무게와 면적이 더
  작게 읽힌다.
- hit box는 동일하게 유지하고 Collapse All glyph만 약 13–13.5pt로
  optical 보정하는 편이 적절하다.

현재 작업 트리 재검증:

- 단일 glyph만 키우는 방식은 폐기하고 네 Project action을 함께
  optical 보정했다. 최종 1× ink box는 New File `11×13`, New Folder
  `16×12`, Refresh `12×13`, Collapse All `13×13px`다.
- 네 action의 수직 중심은 `y=93.5~94.0`, `Project` text는 `y=93.5`로
  오차가 0.5px 이하다. 서로 다른 가로 폭은 문서·폴더·원형 화살표라는
  SF Symbol의 형상 차이다.
- 선택 pill 내부 icon과 editor jump-bar icon의 중심은 모두 `y=63`이다.

### SUI-033 — 전역 selection background의 애매한 형상

- 사용자가 제공한 Outline 스크린샷에서 선택된
  `Suisei UI audit fixture` row는 약 `y=49..71`, 높이 22px다.
- 소스의 `Radius.row = 6`이 적용되어 corner radius는 6px다.
- 22px row의 진짜 capsule이라면 반경은 11px이어야 하고, Swiss-style
  list highlight라면 2–4px 정도의 절제된 반경이 적절하다. 현재 6px은
  두 문법 사이에 있어 “둥근 네모”처럼 보인다.
- 이 형상은 Outline만의 문제가 아니다. Project tree, SCM graph/rows,
  Git rows와 기본 `HoverRow`도 6px 계열을 재사용한다.
- 권장 selection 문법:
  - list selection/hover: full-width row, radius 4
  - segmented control·tab: `Capsule`
  - 여러 줄 card selection: radius 8 또는 12
  - drop target: list selection과 같은 radius 4 + accent stroke

### SUI-034 — Editor pane 전체 파란 focus border

- `editorColumn`은 focused pane 전체에 corner radius 0의
  `1.5pt accent strokeBorder`를 그린다.
- focus border가 split divider와 겹치는 경계는 raster에서 약 2–3px의
  파란 띠로 보이며, rounded editor island 안에 다시 파란 사각형이
  들어가 selected web card 같은 인상을 준다.
- pane header에는 이미 focused 상태일 때 1.5pt accent bottom rule,
  accent icon과 semibold title이 있다. 전체 body stroke는 같은 정보를
  중복 전달한다.
- 권장 focus 문법:
  - pane 전체 accent border 제거
  - focused header의 1.5pt bottom rule 유지
  - header에만 매우 옅은 accent tint 사용
  - title·file icon·terminal icon으로 focus 강조
  - divider accent는 hover/drag 중에만 표시
- editor와 terminal pane이 같은 focus 문법을 공유해야 레이아웃 안에서
  pane 종류가 바뀌어도 선택 표현이 튀지 않는다.

### SUI-035 — Full Git Workbench의 고정 3열과 평면적인 정보구조

심각도: **P2**

`Ctrl+Shift+G`로 연 Full Git Workbench를 Status, Log, Branches, Diff,
PRs까지 전환하며 확인했다.

실측·구현:

- 1351×768 캡처에서 Project Navigator가 약 225px를 계속 점유하고,
  Workbench 본문은 약 1126px다.
- Status 본문은 데이터 양과 관계없이 `Changes 28% / History 48% /
  Files 잔여`로 고정된다. raster 기준 약 `314 / 539 / 273px`다.
- 당시 Changes에는 58개 항목이 있었지만 가장 좁은 열에 들어갔고,
  History는 커밋 한 개만 표시하면서 본문의 거의 절반을 사용했다.
- 상단에는 `Status, Log, Branches, Files, Diff, PRs, Issues, Auth,
  Stash` 9개 항목이 같은 위계의 pill로 나열된다.
- Branches는 한 행, Diff는 `(no diff — select a file)`, PRs는 remote/auth
  안내 두 행만 전체 폭에 늘여 놓고 나머지 화면은 빈 캔버스로 남는다.
- `Files`와 `Diff`는 선택 결과인 detail인데도 독립적인 전역 mode로
  분리되어 있다. Diff 화면에는 파일을 선택할 master list가 없다.
- Git 행의 기본 Button action은 선택·상세 열기가 아니라 해당 문자열을
  clipboard에 복사한다. UI의 열 구조가 암시하는 master–detail 흐름과
  실제 동작이 다르다.
- Workbench 전용 26pt footer 아래에 전역 24pt status line이 다시 있어
  하단 상태 영역이 총 50pt로 중복된다.
- Workbench가 열려도 Project Navigator가 유지되어, Status에서는
  Navigator + 3 Git columns라는 네 개의 세로 영역이 생긴다.

권장 구조:

1. Full Workbench 진입 시 Project Navigator를 임시로 접고, 닫을 때 기존
   가시성을 복원한다. 왼쪽은 현재 mode의 Git master list가 맡는다.
2. 상단 1차 mode는 `Changes / History / Branches / Stashes`로 축소한다.
   `PRs / Issues`는 GitHub 그룹으로 분리하고 `Auth`는 overflow 또는
   settings로 이동한다. `Files / Diff`는 전역 mode에서 제거한다.
3. 고정 비율 대신 `master 320–360pt / detail flex(min 520pt) /
   context 260–320pt optional`의 draggable split을 사용하고 폭을
   레이아웃 상태에 저장한다.
4. Changes에서는 왼쪽 staged/unstaged 파일을 클릭하면 가운데 diff가
   열린다. History에서는 commit list → commit diff → changed files의
   2–3단계 구조를 사용한다.
5. Branches·Stashes·PRs·Issues도 list → detail을 공유한다. remote/auth
   부재는 full-width 회색 행 대신 중앙 empty-state card와
   `Add Remote`/`Open Auth` CTA로 표시한다.
6. 행 기본 클릭은 선택과 상세 열기다. Copy Path/Copy SHA는 context
   menu나 secondary action으로 내린다.
7. Workbench footer는 제거하고 전역 status line 하나만 남긴다.

반응형 규칙:

- `≥1200pt`: master + detail + optional context
- `800–1199pt`: master + detail, context는 drawer
- `<800pt`: 단일 pane과 back navigation

상세 치수와 mode별 wireframe, 완료 조건은
[`docs/SUISEI-SWISS-GRID-AUDIT.md`](docs/SUISEI-SWISS-GRID-AUDIT.md)
§12에 기록했다.

### SUI-036 — 탭 바 좌우 fade가 끝 탭을 잠식

심각도: **P2**

- 14pt 안쪽 fade 자체는 필요한 스크롤 affordance다.
- 문제는 chip row가 fade와 같은 경계에서 끝나 첫/마지막 탭이 흐림
  구간 아래에 정지한 점이었다.
- fade를 줄이거나 overflow 때만 켜는 방식은 요구와 반대였다.
- 현재 수정은 fade 깊이를 14pt로 유지하면서 content 양끝에 동일한
  14pt safe inset을 넣어 가시 viewport를 넓힌다. 탭은 스크롤 중
  edge를 통과할 때만 흐려지고, 양끝 정지 상태에서는 전부 보인다.

### SUI-037 — 레이아웃 3단계 전환의 geometry·gesture 결함

심각도: **P1**

재현된 문제:

1. 그룹 grey round가 측정 span보다 좌우 8pt씩 더 커 4pt tab gap을
   넘어 이웃 loose tab과 겹쳤다.
2. frame cache가 재사용되는 slot index를 key로 사용해 통합→그룹
   전환 때 outgoing view의 `onDisappear`가 incoming chip frame을
   지워 grey round가 간헐적으로 사라졌다.
3. 상태 publish에 animation transaction이 없어 일반→그룹 삽입과
   그룹↔통합 matched-geometry morph가 모두 snap했다.
4. 0.6초 시간 debounce만 사용해 0.6초보다 긴 trackpad momentum tail이
   두 번째 downward step으로 인식됐다.

현재 수정 및 실제 확인:

- grey round 폭을 실측 `minX…maxX`와 정확히 같게 만들었다. 그룹
  `README.md + [No Name]` 옆 `Cargo.lock` loose tab과 4pt gap이
  유지되는 screenshot을 확인했다.
- frame cache를 stable document/layout ID로 바꾸고 reverse morph에
  필요한 last-known member geometry를 유지한다.
- fold/unfold/style toggle의 chrome publish를 0.28–0.30초 smooth
  transaction으로 묶고 chip insertion/removal transition과 그룹↔통합
  matched geometry를 추가했다.
- precise scroll은 direct phase와 momentum phase 전체를 한 물리
  gesture로 잠근다. 실제 Computer Use `↑ → ↑ → ↓ → ↓`에서
  `일반 → 그룹 → 통합 → 그룹 → 일반`이 한 단계씩 진행됐다.

### SUI-038 — pane split 방향 누락

심각도: **P2**

- 단일 jump bar와 각 pane header menu가 `Split Above / Split Below`만
  제공해 가로 배치를 만들 수 없었다.
- 두 메뉴 모두 `Left / Right / Above / Below` 네 방향을 제공하도록
  확장했다.
- `Split Left`는 단순 label 반전이 아니라 Core
  `split_focused_before(Axis::Col)`부터 runtime, C ABI, Swift bridge까지
  연결했다.
- 새 pane이 원래 pane의 x=0 쪽에 놓이고 focus를 얻는 Core 회귀
  테스트를 추가했다.

### SUI-039 — 통합 상태의 status 문구 불일치

심각도: **P3**

- 그룹→통합 전환 뒤 탭은 `layout 1` 한 개로 바뀌지만 상태줄은 이전
  `Grouped into a layout tab · scroll up to unify · down to unfold`을
  계속 표시한다.
- 기능 상태 전이는 정상이나 현재 단계를 잘못 안내한다.
- style toggle 시 Core message도 함께 갱신하도록 수정했다.
- 최신 패키지에서 통합은 `Layout unified · scroll down to show member
  tabs`, 통합→그룹은 `Layout group expanded · scroll up to unify · down
  to unfold`로 현재 단계와 다음 gesture를 정확히 안내하는 것을 확인했다.

## 6. 정상 또는 부분 정상으로 확인된 기능

| 기능 | 결과 | 비고 |
| --- | --- | --- |
| 프로젝트 폴더 열기 | 정상 | 전체 트리, Git branch 표시 |
| 기본 ASCII 편집 | 정상 | 입력, Undo, Redo, Save 확인 |
| 문서 탭 전환·닫기 | 정상 | 호버 `x` 실제 대상 탭 닫힘, 삭제 뒤 stable-ID 재배치 확인 |
| 편집기 한국어 조합 표시 | 부분 정상 | AppKit/Core 회귀 테스트 통과, Computer Use로 실제 marked-text 조합 재현 불가 |
| Project Filter 한국어 IME | 부분 정상 | 조합 정상, clear ghost text 존재 |
| 프로젝트 전체 검색 | 정상 | ASCII·완성형 한글 검색과 결과 이동 동작 |
| 프로젝트 치환 UI | 미실행 | 파괴적 변경을 피하기 위해 실제 Replace는 수행하지 않음 |
| Command Palette | 부분 정상 | 목록·필터 UI 존재, TUI 잔재·IME·접근성 문제 |
| Outline | 부분 정상 | 이동 정상, 전역 row selection 형상은 SUI-033 |
| Go to File | 부분 정상 | Rust 파일 이동 정상, 파일 종류 누락 |
| Split Left/Right/Above/Below | 정상 | pane header 네 방향, 최대 4 pane, Close Pane 동작 |
| 레이아웃 탭 Fold/Group/Unified/Unfold | 부분 정상 | 실제 `↑↑↓↓` 대칭 3단계·geometry·상태 문구 확인, 발견성은 개선 여지 있음 |
| 열린 파일의 외부 삭제 | 정상 | dirty 탭 warning/취소선 유지, clean 탭 2회 miss 뒤 닫힘; idle tick 연결 |
| Markdown Pretty Preview | 정상 | 한글·일본어·emoji 렌더링, 닫은 뒤 레이아웃 복원 |
| Debug Area 터미널 | 부분 정상 | 명령 실행·다중 셸 정상, cwd 불일치 |
| Terminal pane·탭 | 정상 | `Ctrl+Shift+T` pane 전환과 Terminal 탭 생성은 의도된 layout 모델 |
| 중단점 패널 | 정상 | 현재 줄 추가, 목록 표시, 제거 확인 |
| Issues 패널 | 실패 | 확정 Rust 오류 미표시 |
| Source Control 요약 | 부분 정상 | branch·변경 감지, 행 마우스 조작 실패 |
| Git Workbench | 부분 정상 | Status·Log 표시, 상태·개수 불일치와 SUI-035 정보구조 문제 |
| Settings | 실패 | About만 보이며 사이드바 선택 불가 |
| Recovery | 부분 정상 | 항목 Recover는 되나 편집기로 바로 연결되지 않음 |

## 7. 아직 검증하지 않은 항목

재시작 제한은 해제됐고 최신 패키지의 종료·재실행 및 WAL Recover 항목
저장까지 확인했다. 아래는 여전히 남은 항목이다.

- 재시작 전후 레이아웃 탭·분할 비율·선택 범위를 수치로 비교하는 전체 왕복
- 강제 종료 시점별 WAL/Recovery 전체 왕복
- 앱 업데이트 기능

또한 아래 항목은 파괴적이거나 외부 상태를 바꾸므로 실행하지 않았다.

- Git stage, commit, reset, stash, branch 변경
- GitHub 인증, PR·Issue 작성
- 실제 디버거 세션과 프로세스 실행/중지

## 8. 권장 수정 순서

### 1단계 — 편집 안전성

1. SUI-001 IME save commit 처리
2. 저장 시 UI 문자열과 디스크 문자열 일치 회귀 테스트
3. 자동 저장·탭 이동·종료·Recovery까지 marked text 테스트 확대

### 2단계 — 창과 입력 라우팅

1. SUI-002 주기적 트래픽 라이트 프레임 경쟁 제거
2. SUI-006 편집기 전용 window chrome을 editor identifier로 엄격히 한정
3. SUI-009 focused text control 우선 단축키 라우팅
4. SUI-003/SUI-028 Find와 Command Palette를 완전한 `NSTextInputClient` 또는 네이티브 TextField 기반으로 전환

### 3단계 — 핵심 도구 정상화

1. SUI-004 Find next/accept 상태 통합
2. SUI-030 탭 close hit target을 탭 activate Button과 분리
3. SUI-007 LSP diagnostic 연결 상태와 Issues 갱신
4. SUI-005 Settings selection
5. SUI-008 Source Control 행 action
6. SUI-035 Git Workbench를 선택→diff의 master–detail 구조로 개편

### 4단계 — 상태 일관성

1. SCM과 Git Workbench snapshot 통합
2. 터미널 프로젝트 cwd 정책 통일
3. 프로젝트 전환 시 탭 소속·루트 정책 명시
4. 검색 clear, Preview status, Settings branding 정리

### 5단계 — 기능 발견성

1. Editor 메뉴에 Save/Fold Layout, Unfold Layout 추가
2. 레이아웃 탭 첫 사용 안내
3. 중복 View 메뉴 이름 분리
4. New File의 표준 인라인 rename 흐름 제공
5. Command Palette의 TUI 명령을 macOS 명령 체계로 교체하고 접근성 행 제공

## 9. 추천 회귀 테스트

1. `한글` 조합 중 `⌘S` 후 디스크가 정확히 `한글`인지 확인
2. 일본어 변환 중 저장·탭 이동·앱 종료 후 문자열 보존
3. 메인 창을 10초 촬영해 트래픽 라이트 좌표가 0.5pt 이내인지 확인
4. Settings를 열어도 editor 전용 titlebar overlay가 Settings에 나타나지 않는지 확인
5. 모든 TextField에서 `⌘A`, `⌘C`, `⌘V`, Undo가 편집기 상태를 바꾸지 않는지 확인
6. Find의 버튼, `⌘G`, `⇧⌘G`, 위/아래 키가 같은 결과 인덱스를 공유하는지 확인
7. rust-analyzer 진단 publish 후 Issues와 인라인 표시가 동일한지 확인
8. Source Control과 Git Workbench의 branch·change count가 동일한 snapshot인지 확인
9. 새 셸 3개가 모두 프로젝트 루트에서 시작하는지 확인
10. Fold → Merge → Group → Unfold 전 과정에서 각 pane의 파일명이 유지되는지 확인
11. 탭 strip에서 한 번의 trackpad gesture가 정확히 한 단계만 바꾸고
    `일반 → 그룹 → 통합 → 그룹 → 일반`을 왕복하는지 확인
12. grouped run 양옆에 loose tab을 둬 grey round가 4pt tab gap을
    침범하지 않고, 그룹↔통합 왕복 뒤에도 사라지지 않는지 확인
13. 모든 일반·dirty·레이아웃 탭의 호버 `x`가 한 번의 클릭으로 대상 stable ID만 닫는지 확인
14. Workbench에서 Changes 파일을 한 번 클릭하면 같은 snapshot의 diff가 열리고, mode 전환 뒤에도 selection이 유지되는지 확인
15. 800/1200/1600pt Workbench에서 열 폭·empty state·하단 status가 반응형 계약을 지키는지 확인
16. Project Filter가 focused인 상태에서도 Workbench의 `Esc back`이 한 번에 동작하는지 확인

## 10. Swiss Grid 실측 요약

1199×768 기준 editor, Debug Area, Files palette와 780×454 Welcome을
실측했다. 트래픽 라이트 최종 보정은 새 바이너리 교체 후 다시 측정했다.
전체 좌표표,
spacing literal 분포, 권장 토큰과 회귀 측정 계약은
[`docs/SUISEI-SWISS-GRID-AUDIT.md`](docs/SUISEI-SWISS-GRID-AUDIT.md)에
기록했다.

핵심 결과:

- 현재 UI는 사실상 `2pt 원자 + 4pt 본문 + 8pt 거시` 격자를 사용한다.
- 6px panel gap, 48px top band, 24px status bar는 코드와 raster가
  일치한다.
- editor content는 608px이며 현재 2분할에서 304px씩 정확히 나뉜다.
- Files palette 중심은 editor 중심과 1px 이내로 일치한다.
- 숫자가 나쁜 것이 아니라 spacing/height가 semantic token 없이 직접
  입력되어 8/10/12px inset과 22/24/26/28px chrome이 역할별 계약 없이
  섞인 것이 문제다.
- status filename은 pane header의 icon/title/code 축 중 어느 곳에도
  정확히 맞지 않고 약 6px의 애매한 오프셋을 가진다.
- source의 palette layout 폭은 540pt지만 보이는 glass surface는 약
  506px로 측정되어 layout frame과 painted surface의 일치 여부를
  확인해야 한다.
- Navigator `Project` 잉크가 선택 pill보다 1px 왼쪽에서 시작하고,
  Collapse All glyph는 동일한 12pt 설정에서도 optical size가 작다.
- 전역 list selection은 높이 22px에 radius 6px를 사용해 capsule과
  사각 highlight 사이의 애매한 형상이 된다. list는 radius 4,
  segmented/tab만 true capsule로 분리하는 편이 적절하다.
- focused editor pane 전체의 1.5pt 파란 테두리는 제거하고, 이미 존재하는
  header bottom rule·아이콘·title tint로 focus를 전달하는 편이 적절하다.
- SUI-002 트래픽 라이트 진동을 해결하지 않으면 상단의 기준 y축을
  안정적으로 유지할 수 없다.
