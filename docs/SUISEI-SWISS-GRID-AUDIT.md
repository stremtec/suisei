# Suisei Swiss Grid 실측 감사

- 감사 일자: 2026-07-30
- 실행 대상: Suisei 0.1.0 대화형 감사 빌드
- 시험 프로젝트: `/private/tmp/suisei-ui-audit.0FOBuW`
- 기준 화면: Navigator + 2열 editor pane + Inspector, 1199×768 raster
- 추가 화면: Debug Area, Files palette, Welcome
- 관련 사용성 보고서: [`../Report.md`](../Report.md)

## 1. 결론

Suisei는 무질서하게 배치된 UI는 아니다. 실제로는 다음 세 격자가 이미
암묵적으로 존재한다.

1. **2pt 원자 격자** — 6pt panel gap, 22/24/26/28pt chrome 높이,
   6/8/10/12pt inset을 설명한다.
2. **4pt 본문 리듬** — 목록, 텍스트 필드, 섹션 사이의 기본 간격에 적합하다.
3. **8pt 거시 격자** — 24pt status bar, 48pt top band, 304pt pane 폭,
   608pt editor content 폭처럼 큰 구조를 설명한다.

가장 큰 문제는 숫자 자체보다 이 체계가 이름 있는 토큰과 정렬 계약으로
선언되지 않았다는 점이다. 같은 위계에서 8/10/12pt inset과
22/24/26/28pt 높이가 직접 입력되어 있고, status line·pane header·editor
text가 서로 다른 x축을 사용한다. 트래픽 라이트의 지속적인 수직 이동은
상단 격자 전체를 불안정하게 만들어 이 문제를 더 크게 보이게 한다.

반대로 현재 2분할 화면의 핵심 구조는 상당히 좋다.

- editor content 608px가 304px + 304px로 정확히 양분된다.
- editor와 Inspector 사이 6px gap이 코드의 `panelGap`과 일치한다.
- Files palette 중심은 editor 중심과 1px 이내로 일치한다.
- Welcome의 60:40 분할과 왼쪽 44px action margin은 명확하다.

따라서 전면 재배치보다 **기존 2pt 기반을 공식화하고, 정렬축과 예외를
고정하는 작업**이 적절하다.

## 2. 측정 방법과 오차

Computer Use로 실행 중인 앱을 조작하고 각 상태의 전체 창 스크린샷을
수집했다. 경계는 JPEG 원본 픽셀, focus ring, panel fill, separator와
shadow의 색 변화로 판독했다. 그 뒤 SwiftUI의 선언값과 교차 검증했다.

- 좌표 원점: 창 raster의 좌상단 `(0, 0)`
- 범위 표기: `[시작, 끝)`; 끝 좌표는 포함하지 않는다.
- 오차: anti-aliasing, 1.5pt stroke, shadow 때문에 보이는 경계는 `±1px`
- 기준 이미지 폭은 1199px이다. 캡처 계층의 우측 1px 절삭 가능성이 있어
  1200pt 창의 논리 좌표와 직접 동일하다고 가정하지 않았다.
- shadow는 layout gap에 포함하지 않았다. nominal surface edge와 shadow
  falloff를 구분했다.
- 실행 바이너리와 현재 소스의 glass intrinsic size처럼 차이가 날 수 있는
  항목은 `실측`과 `선언`을 따로 기록했다.

## 3. Main Editor 좌표 지도

기준 화면의 핵심 x축은 다음과 같다.

```text
x  0    6                         319             623             927   933                1199
   │gap│<------ Navigator ------->│<-- Pane A 304 ->│<-- Pane B 304 ->│gap│<-- Inspector -->│
   │   │                           └──── editor content 608 ─────────┘   │
   │   └──── editor island는 Navigator 아래에도 계속됨 ─────────────────┘
```

수직 구조는 다음과 같다.

```text
y  0                  48    약 50        76                         약 740  744          768
   │<-- top band 48 -->│gap/edge│header 26│<------ editor body ------>│shadow│status 24│
```

### 3.1 전역 영역

| 오브젝트 | 실측 raster | 선언/계산 | 판정 |
| --- | ---: | ---: | --- |
| 창 | `0,0,1199,768` | 논리 창 약 1200×768 | 기준 |
| top band | `y=0..48` 기준 | `48` | 8pt 거시 격자 일치 |
| Navigator card | `x=6..319`, `y=6..762` | edge gap `6` | 좌·상·하 6px 일치 |
| editor content | `x=319..927` | Navigator 폭만큼 leading inset | 폭 `608 = 76×8` |
| Pane A | `x=319..623` | 50% split | 폭 `304 = 38×8` |
| Pane B | `x=623..927` | 50% split | 폭 `304 = 38×8` |
| editor→Inspector gap | `x=927..933` | `panelGap = 6` | 정확히 일치 |
| Inspector column | `x=933..1199` | persisted live width | 폭 약 266 |
| status line | `y=744..768` | `24` | 8pt/4pt 리듬 일치 |
| Navigator bottom gap | `y=762..768` | `6` | panel gap 일치 |

Navigator는 일반적인 고정 sidebar column이 아니다. editor island가 창
왼쪽까지 계속되고 Navigator card가 그 위에 뜬다. 따라서 `x=319`에는
별도 gutter가 없고 Navigator의 border와 shadow가 editor content와의
분리를 담당한다. 이것은 현재 구현의 의도와 raster가 일치한다.

### 3.2 상단 chrome

| 오브젝트 | 크기/거리 | 비고 |
| --- | ---: | --- |
| traffic light 유효 ink leading | `x≈19pt` | card `x=6`, 좌·우 optical gap 각 13pt |
| traffic light 기준 중심 y | `24pt` | 48pt band의 중심 |
| top toolbar 일반 아이콘 box | `28×24pt` | `ToolbarPlainIcon` |
| Navigator toggle box | 약 `33×29pt` | 15.5pt 아이콘에 따라 일반 box보다 커짐 |
| document tab chip | 높이 `24pt` | 좌우 inset 10, 내부 gap 5 |
| tab strip | 높이 `26pt` | tab 간 gap 4, row 좌우 padding 2 |
| tab close slot | `14×14pt` | hover 때만 표시 |
| trailing toolbar | control 간 `2pt`, 우측 `10pt` | search/settings/Inspector |

감사 시작 빌드에서는 SUI-002 때문에 기준 중심선 `y=24`가 유지되지
않았다. 현재 작업 트리는 20Hz guard, accessory, 실제 표준 버튼 및
private-container frame 쓰기를 모두 제거했다. native standard-button
cell overlay를 Auto Layout으로 한 번 고정하고 실제 표준 버튼은 숨긴다.
새 빌드에서 traffic light·sidebar toggle·trailing tool은 `y≈23.5~24`의
공통 축을 유지했고, 10초 시간차 및 zoom→restore 왕복에서도 이동하지
않았다. 중간에 상단 SwiftUI 행을 28pt로 축소한 시도는 전 위젯을
`y≈14`로 끌어올려 폐기했다.

Navigator toggle만 약 33×29pt인 것은 아이콘 크기에 따라 hit box까지
증가시키는 식 때문이다. 일반 아이콘과 중심은 맞아도 외곽 크기는 홀수
격자로 벗어난다. 큰 glyph가 필요하다면 glyph만 키우고 hit box는
28×24 또는 32×28처럼 명시적으로 고정하는 편이 예측 가능하다.

### 3.3 pane chrome과 editor text

| 오브젝트 | 값 | 정렬축 |
| --- | ---: | --- |
| pane path header | 높이 `26pt` | 2pt 원자 격자 |
| pane header 좌우 inset | `8pt` | pane edge + 8 |
| pane header 내부 gap | `6pt` | icon → title → breadcrumb |
| pane `+` box | `20×18pt` | 감사 당시 trailing |
| pane `×` box | `18×18pt` | 감사 당시 focused pane에서만 표시 |
| split divider hit zone | `7pt` | 1px seam 양쪽 3px를 대칭으로 확보 |
| split divider hover ink | `3×26pt` | 세로 split 기준 |
| editor code 시작 | pane edge + 약 `42px` | 동적 gutter |
| editor focus ring | `1.5pt` | JPEG에서 2–3px band로 보임 |

`26pt` header와 `7pt` divider는 4pt 격자에는 맞지 않지만 모두 이유가
있는 2pt/중심선 예외다. 26pt를 무조건 24 또는 28로 바꾸기보다
`paneHeader = 26`, `dividerHit = 7`을 semantic token으로 선언하는 편이
안전하다. divider 7은 `3 + 1 + 3`의 대칭 hit target이므로 홀수 예외로
남겨야 한다.

focused pane의 1.5pt ring과 divider가 겹치며 정지 화면에서 seam이
약 3px의 파란 띠로 보인다. split boundary의 정보량에 비해 강조가 강하고,
rounded editor island 안에 다시 파란 사각형이 들어가 selected web card
같은 인상을 준다.

pane header에는 이미 focused 상태일 때 다음 표현이 존재한다.

- 1.5pt accent bottom rule
- accent file/terminal icon
- semibold title
- focused editor background

따라서 전체 pane body를 두르는 accent stroke는 중복이다. full border를
제거하고 header bottom rule과 매우 옅은 header tint만 남기는 것이
적절하다. divider accent는 hover 또는 drag 중에만 나타나야 한다.

구현 갱신(2026-07-30): 전체 pane accent stroke를 제거했다. 기존 `+`는
24×24pt `split ▾`로 교체하고 `Split Above / Split Below` menu를
연결했다. 단일 pane jump bar에도 같은 split control을 추가했으며,
분할 상태의 모든 pane에는 24×24pt `×`를 hover와 무관하게 표시한다.

### 3.4 Navigator·Inspector·status의 로컬 축

| 영역 | 대표 leading | panel edge로부터 |
| --- | ---: | ---: |
| Navigator title/mode content | 약 `x=16` | `+10` |
| pane header icon | 약 `x=327` | pane `+8` |
| pane code | 약 `x=361` | pane `+42` |
| Inspector mode/content | 약 `x=943` | Inspector `+10` |
| status filename | 약 `x=337` | Navigator trailing `+18` |

Navigator와 Inspector는 `+10`, pane header는 `+8`이라는 서로 다른 로컬
격자를 사용한다. 이것만으로는 오류가 아니다. 문제는 status filename이
pane icon 축과 title 축 중 어느 쪽에도 정확히 맞지 않는다는 점이다.
현재 status leading은 다음 식이다.

```text
navW + panelGap + 7 + 12
```

결과적으로 Navigator trailing에서 약 18px 떨어지고 pane header의
대표 축과 약 6px 어긋난다. status filename을 pane icon leading,
pane title leading, 또는 code leading 중 하나에 의도적으로 맞춘 뒤 그
선택을 토큰으로 고정해야 한다.

### 3.5 사용자 제공 Navigator·Outline 확대 스크린샷

2026-07-30 09:04:20, 09:05:06 확대 스크린샷을 별도로 실측했다.

#### Navigator heading과 action

| 오브젝트 | 보이는 ink/surface box | 판정 |
| --- | ---: | --- |
| 선택된 파란 Navigator pill | `x=21..73`, 52×24px | 기준 surface |
| `Project` text ink | `x=20..56`, 36×10px | pill보다 1px 왼쪽 |
| New File glyph | 최종 11×13px | 28×24 hit box 유지 |
| New Folder glyph | 최종 16×12px | 폴더 형상상 넓은 비율 유지 |
| Refresh glyph | 최종 12×13px | optical size 확대 |
| Collapse All glyph | 최종 13×13px | optical size 축소·중심 보정 |

`Project` row와 mode strip은 둘 다 horizontal padding 10을 사용하지만
선택 pill은 `NavStrip.inset = 2` 안쪽에서 다시 그려진다. container의
수학적 edge는 같아도 실제 파란 surface와 text ink의 시작점은 다르다.
title leading을 1–2pt 안으로 이동하거나 선택 pill의 geometry anchor를
공유해야 한다.

동일한 font size가 동일한 optical size를 보장하지 않으므로 네 action을
각각 보정했다. 최종 수직 중심은 `y=93.5~94.0`, Project text는 `y=93.5`로
오차가 0.5px 이하다. Navigator 선택 pill 내부 icon과 editor jump-bar
icon의 최종 중심도 모두 `y=63`이다.

#### Outline selection shape

선택된 `Suisei UI audit fixture` row는 nominal하게 약
`x=38..338`, `y=49..71`, 높이 22px다. source의 corner radius는
`Radius.row = 6`이다.

```text
현재:  row height 22 / radius 6
pill:  row height 22 / radius 11
list:  row height 22 / radius 2–4
```

현재 값은 true capsule도 아니고 Swiss-style rectangular list highlight도
아니다. 사용자가 “애매한 둥근 네모”라고 느낀 원인이 수치상으로도
확인된다.

이 문법은 Outline에만 국한되지 않는다.

- `ProjectTreeView` selection과 drop target
- SCM graph와 SCM row
- Git change row
- 기본 `HoverRow(corner: 6)`
- 일부 field와 badge

목록의 selection/hover는 radius 4의 절제된 full-width row로 통일하고,
Navigator mode·Inspector mode·document tab처럼 segmented selection인
경우에만 실제 `Capsule`을 사용해야 한다. 여러 줄 card selection은
radius 8 또는 12로 별도 분리한다.

## 4. Debug Area

동일한 1199×768 창에서 Navigator의 Debug Area 버튼을 열고 닫아 측정했다.
앱은 종료하거나 재시작하지 않았다.

| 오브젝트 | 실측 | 선언/설명 |
| --- | ---: | --- |
| dock 시작 | 약 `y=330` | live `debugAreaH`에 따라 변동 |
| dock 끝 | 약 `y=740` | editor island 하단 |
| 보이는 dock 높이 | 약 `410px` | persisted live height |
| terminal header | 약 `28px` | 선언 `28pt` |
| header→grid separator | header 시작 + 약 `28px` | card background가 그림 |
| terminal content leading | editor content leading과 동일 | Navigator 아래 band는 card 전체 폭 |

좋은 점은 Debug Area header가 28pt이고 pane header 26pt와 인접한
2pt 단계로 구성된다는 것이다. terminal의 tint band는 Navigator 아래까지
editor island 전체 폭으로 이어지므로 `x=319`에서 색이 갑자기 끊기지 않는다.

개선할 점은 26pt pane header, 28pt terminal header, 24pt toolbar/tab,
22pt Inspector segment가 같은 chrome family인데도 역할별 height token이
명시되지 않았다는 것이다. 한 높이로 강제 통일할 필요는 없지만 다음처럼
계층을 선언해야 한다.

```text
chrome.control = 22
chrome.toolbar = 24
chrome.paneHeader = 26
chrome.dockHeader = 28
```

## 5. Files palette

| 항목 | 실측 | 선언/판정 |
| --- | ---: | --- |
| 보이는 glass surface | 약 `x=371..877` | 폭 약 506px |
| 선언 layout frame | `540pt` | visible glass와 약 34px 차이 |
| 보이는 top | 약 `y=97` | 코드의 top padding 72 + 창/safe-area 영향 |
| 보이는 bottom | 약 `y=489` | 높이 약 392px |
| palette 중심 | 약 `x=624` | editor 중심 약 `x=623`, 오차 1px |
| panel content inset | `16pt` | title·query |
| list inset | 외부 `8pt` + row `12pt` | 2pt 격자 |
| list row spacing | `2pt` | compact list |

가장 중요한 editor-relative centering은 정상이다. Navigator와 Inspector의
폭이 달라도 palette 중심이 window center가 아니라 editor center를
따른다.

반면 `.frame(width: 540)` 선언과 실제로 보이는 glass surface 폭은
약 506px로 다르다. rounded corner의 top edge가 아니라 panel 중간
`y=300`에서 측정한 값이므로 단순 corner clipping만으로 설명되지 않는다.
`glassPanel`에 frame을 적용하는 modifier 순서나 intrinsic width를 확인해
layout frame과 painted surface가 동일한 폭을 갖게 해야 한다. 의도된
visible width가 506이라면 토큰을 508 또는 504로 명시하고 540 wrapper를
없애는 편이 낫다.

## 6. Welcome

Welcome은 780×454 고정 창이며 60:40 column 구조가 분명하다.

```text
x 0                              468                         780
  │<------ Brand 60% ------------>│<------ Recents 40% ------>│
```

| 오브젝트 | 실측/선언 | 판정 |
| --- | ---: | --- |
| 창 | `780×454` | 고정 |
| split | `x=468` | 정확히 60:40 |
| seam | `1px` | tonal split + hairline |
| 왼쪽 action block | `x=44..424` | 폭 380, 좌우 44 |
| action row | 약 `38px` | font + 상하 11 |
| action row gap | `10px` | 2pt 격자, 4/8pt 리듬에는 비정형 |
| 마지막 action bottom margin | `40px` | 5×8 |
| 오른쪽 row block | `x=486..762` | pane edge +18, 우측 18 |
| `Recents` heading leading | pane edge +28 | row 축보다 10px 안쪽 |
| close control inset | `14×14pt` 기준 | 좌상단 14 |

60:40 split, 44px action margin, 40px bottom margin은 강한 구조를 만든다.
다만 source comment의 “golden-ratio card”와 실제 `780/454 ≈ 1.718`은
일치하지 않는다. 문구를 실제 60:40 Xcode-style launch sheet로 정정하거나,
정말 황금비가 목표라면 높이를 약 482px로 바꿔야 한다.

`Recents` heading은 row보다 10px 안쪽에 위치한다. 의도적인 hanging
indent라면 semantic token으로 남기고, 그렇지 않다면 heading과 row의
leading을 하나로 맞춰야 한다. action row gap 10px 역시 2pt 격자에는
맞지만 8pt 거시 리듬을 강조하려면 8 또는 12로 선택할 수 있다.

## 7. 소스 spacing 분포

`suisei-app/Suisei/*.swift`의 숫자 literal padding과 H/V stack spacing을
집계했다. 식으로 계산한 값과 동적 값은 제외했다.

### 7.1 padding

총 211개다.

| 값 | 횟수 | 값 | 횟수 | 값 | 횟수 |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 4 | 2 | 16 | 3 | 7 |
| 4 | 27 | 5 | 6 | 6 | 22 |
| 7 | 6 | 8 | 34 | 9 | 2 |
| 10 | 28 | 11 | 3 | 12 | 21 |
| 14 | 6 | 16 | 9 | 18 | 6 |
| 20 | 3 | 24 | 3 | 28 | 5 |
| 40 | 1 | 44 | 1 | 46 | 1 |
| 72 | 1 |  |  |  |  |

- 2pt 격자 일치: `183/211 = 86.7%`
- 4pt 격자 일치: `104/211 = 49.3%`
- 홀수 padding: `28/211 = 13.3%`

### 7.2 HStack/VStack spacing

총 135개다.

| 값 | 횟수 | 값 | 횟수 | 값 | 횟수 |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | 44 | 1 | 7 | 2 | 12 |
| 3 | 9 | 4 | 12 | 5 | 4 |
| 6 | 22 | 8 | 11 | 10 | 10 |
| 11 | 1 | 12 | 1 | 16 | 2 |

- 2pt 격자 일치: `114/135 = 84.4%`
- 4pt 격자 일치: `70/135 = 51.9%`
- 홀수 spacing: `21/135 = 15.6%`

이 분포는 8pt-only 체계로 갈아엎어야 한다는 증거가 아니다. 반대로
macOS compact chrome에 맞는 2pt atom이 이미 존재한다는 증거다. 홀수 값
중 1pt separator, 3pt git stripe, 5/7pt optical 또는 symmetric hit area는
예외로 허용하고, 그 밖의 홀수 raw literal만 제거하는 것이 현실적이다.

## 8. 권장 Swiss Grid 토큰

숫자 크기보다 역할을 이름으로 고정한다.

```swift
enum Grid {
    static let atom: CGFloat = 2
    static let rhythm: CGFloat = 4
    static let macro: CGFloat = 8

    enum Space {
        static let hairline: CGFloat = 1
        static let tight: CGFloat = 2
        static let compact: CGFloat = 4
        static let panelGap: CGFloat = 6
        static let controlInset: CGFloat = 8
        static let panelInset: CGFloat = 10
        static let sectionInset: CGFloat = 12
        static let large: CGFloat = 16
        static let sectionGap: CGFloat = 24
        static let topBand: CGFloat = 48
    }

    enum Height {
        static let inspectorSegment: CGFloat = 22
        static let toolbarControl: CGFloat = 24
        static let paneHeader: CGFloat = 26
        static let dockHeader: CGFloat = 28
        static let status: CGFloat = 24
    }

    enum Selection {
        // Project tree, Outline, SCM, Git list.
        static let listRadius: CGFloat = 4
        // Multi-line result cards only.
        static let cardRadius: CGFloat = 8
        // Segmented controls and tabs use Capsule(), not a numeric radius.
    }

    enum PaneFocus {
        static let headerRule: CGFloat = 1.5
        // Full-body accent stroke is intentionally absent.
    }

    enum Exception {
        // 3 + 1px seam + 3: 중심선 대칭 hit target.
        static let dividerHit: CGFloat = 7
    }
}
```

이 예시는 모든 view를 한 파일의 전역 상수에 묶으라는 뜻이 아니다.
구조 토큰은 공용으로 두고, optical nudge는 해당 component 내부에
`optical` 또는 `exception` 이름으로 남겨야 한다.

## 9. 우선순위별 조치

| 우선순위 | 항목 | 조치 |
| --- | --- | --- |
| P1 | 불안정한 top axis | SUI-002 traffic light frame 경쟁을 먼저 제거 |
| P2 | spacing token 부재 | 2pt atom과 역할별 inset/height token 도입 |
| P2 | status 정렬축 불명확 | pane icon/title/code 중 하나와 명시적으로 정렬 |
| P2 | palette paint/layout 폭 차이 | 540 layout frame과 약 506 visible surface의 원인 제거 |
| P2 | split seam 과강조 | focus ring과 divider ink의 중첩을 한 단계 약화 |
| P3 | Navigator heading optical axis | `Project`를 선택 pill edge와 1px 이내로 정렬 |
| P3 | Project action optical size | 28×24 hit box를 유지하고 네 glyph를 개별 보정 |
| P3 | 전역 selection 문법 | list radius 4, segmented/tab `Capsule`, card 8/12로 분리 |
| P3 | pane focus가 값싼 card처럼 보임 | full blue border 제거, header rule·tint·title로 focus 전달 |
| P3 | Welcome local grid | heading 28 vs row 18 indent를 의도된 토큰으로 선언 또는 통일 |
| 유지 | 6px panel gap | window/card 및 editor/Inspector에서 일관되게 유지 |
| 유지 | editor-relative palette center | 1px 이내 정렬을 회귀 테스트로 고정 |
| 유지 | 608→304+304 split | 비율·divider 수정 뒤에도 보존 |
| 유지 | Welcome 60:40 | 문서의 golden-ratio 설명만 실제 수치와 맞춤 |

## 10. 회귀 측정 계약

최소 두 viewport에서 screenshot 또는 geometry test를 고정한다.

1. 1200×768 compact editor
2. 1600×1024 wide editor

각 화면에서 다음을 자동 측정한다.

- `panelGap = 6 ± 1px`
- `topBand = 48 ± 1px`
- `status = 24 ± 1px`
- `paneHeader = 26 ± 1px`
- `debugHeader = 28 ± 1px`
- editor→Inspector nominal gap `6 ± 1px`
- 50:50 split일 때 두 pane 폭 차이 `≤ 1px`
- Files palette 중심과 editor content 중심 차이 `≤ 1px`
- Navigator heading ink와 선택 pill leading 차이 `≤ 1px`
- Navigator action glyph의 optical height 차이 `≤ 2px`
- 20–24px 단일 행 list selection radius `4px`
- segmented/tab selection은 높이의 절반 반경 또는 `Capsule`
- focused pane body에는 상시 accent 외곽선이 없고 header rule만 표시
- pane divider accent는 hover/drag 중에만 표시
- 10초 연속 캡처에서 traffic light 중심 y 변화 `≤ 0.5pt`
- nav/Inspector live resize 후에도 pane 최소 폭과 local leading 축 유지

스크린샷만으로는 display scale과 shadow 때문에 false positive가 생길 수
있다. 가능하면 SwiftUI `GeometryReader`/AppKit frame을 테스트용 snapshot에
함께 기록하고, raster는 최종 visual diff로 사용한다.

## 11. 소스 교차 확인 지점

- `ContentView.swift`
  - `panelGap = 6`, `topBandHeight = 48`, `statusBarHeight = 24`
  - `editorCard`, `sidebarColumn`, `inspectorColumn`
  - `panePathBar`, `splitEditorLayout`, `SplitDivider`
  - `debugArea`, `paletteOverlay`, `statusLine`
- `GlassChrome.swift`
  - `ToolbarPlainIcon`, `ToolbarTabChip`
- `EngineBridge.swift`
  - `EditorMetrics.gutter`, `gutterTextGap`
- `WelcomeView.swift`
  - `windowSize = 780×454`, `brandSplit = 0.6`
  - action block 44px inset, Recents 18/28px local insets

## 12. Full Git Workbench 재설계

### 12.1 현재 화면 실측

`Ctrl+Shift+G`로 연 1351×768 상태에서 Workbench의 실질적인 본문 폭은
약 1126px였다. 왼쪽 Project Navigator 약 225px가 그대로 남아 있기
때문이다.

```text
x  0                  225               539                         1078             1351
   │ Project Navigator │ Changes ≈314px │ History ≈539px           │ Files ≈273px    │
   │<---- 16.7% ------>│<-- WB 28% ---->│<------- WB 48% --------->│<-- remainder -->│
```

| 오브젝트 | 현재 값 | 판정 |
| --- | ---: | --- |
| Workbench toolbar | 40pt | 높이는 compact하나 9개 pill이 한 행에 과밀 |
| Workbench 본문 | 약 1126px | Navigator와 함께 네 개 세로 영역 생성 |
| Changes | `max(180, width × 0.28)` | 데이터 58개인데 가장 좁음 |
| History | `max(220, width × 0.48)` | 커밋 1개인데 가장 넓음 |
| Files | remaining | 선택 전에는 사실상 빈 열 |
| Workbench footer | 26pt | 전역 status 24pt와 중복, 합계 50pt |
| list selection | radius 6 | §3.6의 전역 selection 문법과 불일치 |

고정 비율은 “각 열이 담는 데이터가 항상 비슷하다”는 전제를 갖지만 실제
Git 작업은 그렇지 않다. Changes는 파일 수에 따라 폭발하고, History의
선택 commit이 없으면 Files는 비어 있으며, Branches·Diff·PRs는 단일
surface로 바뀌어 같은 열 구조조차 사용하지 않는다.

### 12.2 정보구조

현재 top mode는 서로 다른 위계를 평평하게 만든다.

- 작업 공간: Status, Log, Branches, Stash
- 선택 결과: Files, Diff
- GitHub 작업: PRs, Issues
- 환경 설정: Auth

권장 위계:

```text
Git Workbench
├─ Git: Changes · History · Branches · Stashes
├─ GitHub: PRs · Issues
└─ Utilities: Auth/Accounts · Refresh · More
```

`Files`와 `Diff`는 mode가 아니라 현재 선택의 detail이다. `Auth`는 매일
오가는 작업 공간이 아니라 연결 설정이므로 primary navigation에서
제외한다. 상단은 branch selector, 두 개의 명확한 mode group, sync,
refresh, close만 남긴 44–48pt toolbar가 적절하다.

### 12.3 공통 master–detail grid

고정 percentage 대신 semantic pane과 min/max를 선언한다.

| 영역 | 권장 폭 | 역할 |
| --- | ---: | --- |
| master | 기본 336pt, min 280, max 480 | file/commit/branch/PR 목록과 filter |
| divider | visual 1pt, hit 7pt | drag resize |
| detail | flex, min 520pt | diff, commit, branch, PR 본문 |
| context | 기본 288pt, min 260, max 320 | changed files, metadata, checks; optional |

Project Navigator는 Full Workbench 진입 시 임시로 접고 닫을 때 기존
visibility를 복원한다. 그렇지 않으면 master가 두 개 생기고 Workbench가
세 열인 상태에서는 화면이 네 개의 수직 조각으로 분해된다. 사용자가
Navigator를 명시적으로 다시 열 수는 있어야 한다.

반응형 규칙:

| 사용 가능 폭 | 배치 |
| --- | --- |
| `≥1200pt` | master + detail + optional context |
| `800–1199pt` | master + detail, context는 trailing drawer |
| `<800pt` | 한 pane, selection 후 detail로 push하고 Back 제공 |

사용자가 조절한 master/context 폭은 레이아웃 상태에 저장하되 viewport
축소 시 min width를 우선하고, 다시 넓어졌을 때 마지막 유효 폭을 복원한다.

### 12.4 mode별 화면

| Mode | Master | Detail | Optional context |
| --- | --- | --- | --- |
| Changes | staged/unstaged 파일, commit field/actions | 선택 파일 diff | 파일 metadata/hunks |
| History | commit list | commit message + diff | changed files |
| Branches | local/remote branch list | 최근 commit·ahead/behind | upstream/actions |
| Stashes | stash list | stash diff | affected files/actions |
| PRs | PR list/filter | PR description + diff/check summary | reviewers/labels/checks |
| Issues | issue list/filter | issue body/timeline | labels/assignees |

파일·commit 행의 primary click은 selection을 만들고 detail을 연다. 현재
구현처럼 전체 문자열을 clipboard에 복사하는 동작은 `Copy Path`,
`Copy SHA` context menu로 이동한다. selection은 mode를 잠깐 전환해도
stable ID 기준으로 유지한다.

### 12.5 empty state와 시각 문법

- `(no diff — select a file)` 같은 100% 폭 회색 행을 만들지 않는다.
- detail 중앙에 icon, 한 줄 title, 짧은 설명, 최대 한 개의 primary CTA를
  둔다.
- remote가 없으면 `Add Remote`, 인증이 없으면 `Open Auth`를 제시한다.
- list selection은 radius 4, mode segment만 true `Capsule`을 사용한다.
- column header는 32pt 높이, 12pt label, leading 12/16pt 축으로 통일한다.
- 일반 UI는 proportional type, path/hash/diff만 monospaced type을 쓴다.
- Workbench 전용 26pt footer를 제거하고 전역 24pt status line 하나만
  유지한다. keyboard hint는 toolbar tooltip 또는 command menu로 옮긴다.
- focused detail 전체를 파란 stroke로 두르지 않고 header tint/rule로
  focus를 표시한다.

### 12.6 데이터 모델과 완료 조건

현재 count는 `String` line을 다시 filter하여 계산하고, 행의 header 여부와
selection도 prefix 문자로 판정한다. 레이아웃을 고칠 때
`GitChangeRow`, `GitCommitRow`, `GitFileRow`, `GitEmptyState` 같은 typed
model로 함께 바꿔야 SUI-020의 count 불일치를 다시 만들지 않는다.

완료 조건:

1. Source Control과 Workbench가 같은 repository snapshot과 count 정의를
   사용한다.
2. Changes 파일 한 번 클릭으로 해당 diff가 열리고 keyboard selection도
   같은 detail 경로를 사용한다.
3. History commit 선택 시 changed files와 diff가 같은 commit ID를
   가리킨다.
4. `Files`와 `Diff`는 top navigation에 존재하지 않는다.
5. Project Navigator의 진입 전 visibility가 Workbench 종료 후 복원된다.
6. 800/1200/1600pt에서 어느 pane도 min width 아래로 찌그러지지 않는다.
7. 빈 상태는 detail 내부에서 중앙 정렬되고 full-width selected row처럼
   보이지 않는다.
8. 하단에는 24pt status line 하나만 존재한다.
9. split width와 selection이 layout save/restore에 포함된다.
10. VoiceOver에 mode, row selection, staged state, diff 대상, CTA가
    의미 있는 label/value로 노출된다.
