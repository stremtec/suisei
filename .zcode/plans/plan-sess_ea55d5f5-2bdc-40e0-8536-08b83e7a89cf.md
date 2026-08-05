## 레이아웃 전환 애니메이션 재설계 — 순차적 단계 분해 (하이브리드 방안)

### 원인
현재 `withAnimation { refreshChrome() }`가 크롬 전체를 한 번에 교체합니다. 전환 시 칩 배열이 바뀌면서(unified 칩 ↔ 개별 칩, stableId 상이) SwiftUI가 geometry 연속성을 잃고, `tabFrames`가 한 프레임 늦게 갱신되어 "튀어 오름 후 정착"이 발생합니다.

### 설계 원칙
각 전환을 **독립된 geometry 변화**로 분해하고, 각 단계가 0.1s 간격으로 겹치면서 시작되도록 합니다. matchedGeometryEffect를 제거하고 컨테이너 bounds를 직접 `withAnimation`으로 제어합니다.

### 6개 전환의 단계 분해

| 전환 | 단계 | 변화 | 트리거 |
|---|---|---|---|
| **일반→그룹** (↑) | ① gather | `gather_folded_docs`가 buffers 순서 변경 → 칩들이 연속 run으로 슬라이드 (stableId 유지, 위치만 이동) | Core `fold_layout` (gather만, style은 아직 Grouped 아님 → 그룹 배경 없음) |
| | ② container (0.1s 후) | 회색 컨테이너가 run 위로 fade-in + scale | Core `set_layout_grouped` (style = Grouped 설정) |
| **그룹→통합** (↑) | ③ merge | 멤버 칩들이 첫 칩 위치로 수렴 (opacity → 0, scale → 0.8) | Core `toggle_layout_style` → Unified |
| | ④ reclaim (0.1s 후) | 빈 공간만큼 뒤 탭들이 앞으로 슬라이드 | Swift에서 offset 보정 (Core 재개입 불필요 — unified 칩이 이미 첫 멤버 자리에 있음) |
| **통합→그룹** (↓) | ④⁻¹ expand | 뒤 탭들이 뒤로 밀리고, 통합 칩 자리가 벌어짐 | Core `toggle_layout_style` → Grouped |
| | ③⁻¹ merge (0.1s 후) | 멤버 칩들이 원래 위치로 펼쳐짐 (opacity → 1, scale → 1) | `tabFrames` 갱신 후 자연 발생 |
| **그룹→일반** (↓) | ②⁻¹ container | 컨테이너 fade-out | Core `unfold_layout` |
| | ①⁻¹ scatter (0.1s 후) | 칩들이 원래 위치로 되돌아감 | buffers 순서 복원 (Core에서 자동) |

### 구현 계층

#### 1. Core (Rust) — `layouts.rs` + `app.rs`
- `fold_layout`을 두 단계로 분해:
  - `gather_only()` — buffers 순서만 재정렬하고 `LayoutTab` 생성하지 않음. 화면은 여전히 분할 상태.
  - `commit_layout()` — `LayoutTab` 생성 + `active_layout` 설정.
- 하지만 **더 단순한 접근**: `fold_layout`이 현재처럼 한 번에 상태를 바꾸되, Swift에서 **시각적으로만** 분해. Core는 단일 상태 전환, Swift가 `tabFrames` 기반으로 순차 에니메이션 구동.

**선택: Core 단일 전환 + Swift 시각적 분해** — Core에 중간 상태를 만들지 않아 FFI가 단순합니다.

#### 2. Swift — `ContentView.swift` + `EngineBridge.swift`

**`EngineBridge.swift` 변경:**
- `advanceLayoutPresentation` / `retreatLayoutPresentation`을 **순차 Task**로 재구성:
  ```swift
  func advanceLayoutPresentation() {
      Task { @MainActor in
          // ① gather: withAnimation으로 칩 슬라이드만
          // (Core fold_layout 호출 → refreshChrome in withAnimation)
          let ok = foldLayout()  // 이미 withAnimation 안에서 refreshChrome 호출
          if ok {
              try? await Task.sleep(nanoseconds: 100_000_000) // 0.1s
              // ② 컨테이너는 tabFrames 갱신 후 자연 등장
              // (이미 Grouped 상태이므로 layoutGroupRuns가 컨테이너 생성)
          }
      }
  }
  ```
- `matchedGeometryEffect` 제거 — 컨테이너와 unified 칩 모두 `groupSpace` namespace에서 제거
- 대신 컨테이너 bounds를 `withAnimation`으로 직접 에니메이트:
  ```swift
  // layoutGroupContainer에서:
  .frame(width: animatingWidth, height: Self.tabLabelFrameH)
  .position(x: animatingCenterX, y: Self.tabLabelFrameH / 2)
  ```
  - `animatingWidth`/`animatingCenterX`는 `tabFrames`에서 계산하되, 전환 중에는 이전 값 유지 (stale 방지)

**`ContentView.swift` 변경:**
- `layoutGroupContainer`에서 `matchedGeometryEffect` 제거, `transition`을 `withAnimation` 기반으로 교체
- `ToolbarTabChip`의 `isLayout` 배경에서 `matchedGeometryEffect(id: tabId, in: groupSpace, isSource: false)` 제거
- 새로운 `@State` 추가:
  - `layoutTransitionPhase: LayoutTransitionPhase` — `.none | .gathering | .container | .merging | .reclaiming`
  - `pendingContainerBounds: CGRect?` — 전환 중 컨테이너 시작 bounds 보존
- `layoutGroupRuns`가 전환 중에는 `pendingContainerBounds`를 우선 사용
- 칩 `.transition`을 phase에 따라 분기:
  - merge 단계: 멤버 칩 removal = `opacity + scale(0.8)` → 첫 칩 위치로 수렴
  - expand 단계: 멤버 칩 insertion = `opacity + scale(0.8 → 1)` → 원래 위치로 펼쳐짐

**핵심 — tabFrames freeze 메커니즘:**
- 전환 시작 시 `tabFrames` 스냅샷을 `pendingContainerBounds`에 저장
- `layoutGroupRuns`이 `pendingContainerBounds`가 있으면 그것을 사용 (빈 frame 문제 회피)
- 전환 완료 후 `pendingContainerBounds = nil`로 해제

#### 3. 애니메이션 상수
```swift
private static let layoutGatherAnimation: Animation = .snappy(duration: 0.22)
private static let layoutContainerAnimation: Animation = .easeOut(duration: 0.18)
private static let layoutMergeAnimation: Animation = .spring(duration: 0.28, bounce: 0.04)
private static let layoutStepDelay: UInt64 = 100_000_000 // 0.1s
```

#### 4. 제거 대상
- `groupSpace` `@Namespace` — matchedGeometryEffect 전체 제거
- `layoutGroupContainer`의 `.matchedGeometryEffect(id:isSource:)` 
- `ToolbarTabChip`의 `isLayout` 배경 `.matchedGeometryEffect(id:isSource: false)`
- `.animation(.snappy(duration: 0.22), value: layoutGroupRuns(...))` — implicit 애니메이션 충돌 원인

### 파일별 변경 요약

| 파일 | 변경 |
|---|---|
| `suisei-app/Suisei/EngineBridge.swift` | `advanceLayoutPresentation`/`retreatLayoutPresentation` → 순차 Task 재구성, `layoutPresentationAnimation` 상수 분할 |
| `suisei-app/Suisei/ContentView.swift` | `layoutGroupContainer` matchedGeometry 제거 + 직접 bounds 에니메이션, `layoutTransitionPhase` 상태 추가, `layoutGroupRuns` freeze 로직, 칩 transition 분기 |
| `suisei-app/Suisei/GlassChrome.swift` | `ToolbarTabChip`의 `groupSpace` matchedGeometryEffect 제거 |

### 검증
- `cargo test --workspace` — Core 변경 없음, 447 tests 유지
- `./scripts/package-suisei-app.sh` — Swift 빌드 + 패키징
- 앱 실행 후 수동 검증: 일반→그룹, 그룹→통합, 통합→그룹, 그룹→일반 4방향 왕복