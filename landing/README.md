# Stology 소개 사이트 (한국어 · 영어)

Claude·ChatGPT를 이미 결제한 Mac 대학생에게 Stology를 소개하는 독립 정적 사이트다. 한국어 `/`와 영어 `/en/` 두 페이지를 만들고, 기능 소개는 실제 앱 UI를 재현한 브라우저 장면으로 보여 준다. 제품 범위는 [제품 정책](../app/docs/product-policy.md), 노트 기능은 [앱 단축키 문서](../app/docs/keyboard-shortcuts.md)를 기준으로 한다. 타깃·카피·구조·채널 계획과 그 근거는 [재구축·마케팅 계획](../decisions/2026-11-23/research/landing-marketing-rebuild-plan.md)과 [벤치마크·채널·도구 조사](../decisions/2026-11-23/research/landing-benchmark-and-channel-research.md)에 있다.

## 로컬 실행

Node.js와 npm이 필요하다. 아래 명령은 저장소 루트 기준이다.

```sh
npm install --prefix landing
npm run dev --prefix landing          # http://127.0.0.1:5173/ 와 /en/
npm run check --prefix landing        # 카탈로그·장면·한영 문구 정적 검사
npm run build --prefix landing        # landing/dist/ (index.html, en/index.html, 404.html)
npm run preview --prefix landing
```

배포 시 환경변수(없으면 해당 기능은 조용히 꺼진다):

| 변수 | 역할 |
|---|---|
| `VITE_SITE_URL` | 공개 주소(예: `https://stology.app`). 있으면 canonical·hreflang(ko/en/x-default)·`og:url`·절대 경로 `og:image`를 넣는다 |
| `VITE_UMAMI_ID` | Umami 웹사이트 ID. 있으면 쿠키 없는 방문 분석 스크립트를 넣고 `download_click`·`lang_switch`·`scene_select`·`faq_open`·`outbound` 이벤트를 보낸다 |
| `VITE_UMAMI_SRC` | Umami 스크립트 주소(기본 `https://cloud.umami.is/script.js`) |

```sh
VITE_SITE_URL=https://example.com VITE_UMAMI_ID=xxxx npm run build --prefix landing
```

빌드 결과 `landing/dist/`를 정적 호스팅의 사이트 루트에 올린다. 자산 URL은 `/` 기준이라 하위 경로 배포에는 조정이 필요하다. `404.html`은 Cloudflare Pages 등에서 그대로 쓰인다. 다운로드 버튼은 GitHub 릴리스의 공증 DMG로 연결되며 새 버전을 게시하면 `src/config.js`의 `DOWNLOAD_VERSION`을 올린다.

## 구조와 동작

| 경로 | 역할 |
|---|---|
| [index.html](index.html) · [en/index.html](en/index.html) | 언어별 문서·검색·공유 메타 정보, `SoftwareApplication` 구조화 데이터 |
| [404.html](404.html) | 없는 주소 안내(한·영) |
| [vite.config.js](vite.config.js) | 두 페이지 입력, `VITE_SITE_URL`에 따른 canonical·hreflang·og 태그 삽입 |
| [src/main.js](src/main.js) | 페이지 조립, 히어로, 섹션 동작, 언어 전환, 대화상자, 분석 이벤트 |
| [src/i18n/ko.js](src/i18n/ko.js) · [src/i18n/en.js](src/i18n/en.js) | 모든 문구. 섹션 카피, 29개 항목 설명, 장면 안의 사용자 데이터(과목·파일·본문·질문), 06 지도 장면의 검색어와 영어 번역표(`scenes.atlas`) |
| [src/catalog.js](src/catalog.js) | 기능 목록(언어 중립): id·분류·단축키·Markdown 대체 입력·핵심 여부 |
| [src/feature-explorer.js](src/feature-explorer.js) | 핵심/모두 보기, 검색·분류, 항목 선택과 장면 재생 |
| [src/scenes/runtime.js](src/scenes/runtime.js) | 타임라인·재생기(화면 밖·백그라운드 정지, 동작 줄이기), 타이핑·선택·키캡 토스트·포인터·프레임 축소 |
| [src/scenes/frames.js](src/scenes/frames.js) | 실제 앱 UI 재현: 문서 카드, PDF 카드, 자료/AI 패널, 왼쪽 사이드바, 지도 카드, 컨텍스트 메뉴·대화상자·메뉴 막대·공유 |
| [src/scenes/ui-labels.js](src/scenes/ui-labels.js) | 앱 화면에 적힌 글자(UI 크롬). 앱이 한국어뿐이므로 영어 페이지에서도 그대로 |
| [src/scenes/note-scenes.js](src/scenes/note-scenes.js) · [pdf-scenes.js](src/scenes/pdf-scenes.js) · [window-loop.js](src/scenes/window-loop.js) | 노트 16·PDF 13 장면과 01 섹션의 앱 창 전체 루프 |
| [src/scenes/atlas-loop.js](src/scenes/atlas-loop.js) | 06 과목 지도 장면: 지도 카드 + 개념과 원본 근거 패널, 조작 타임라인. 따로 받는 청크 |
| [src/scenes/atlas-engine.js](src/scenes/atlas-engine.js) | 앱 지식지도(`app/ui/src/graph.rs`·`graph_math.rs`)의 배치·카메라·강조·이름표를 옮긴 Canvas 2D 렌더러 |
| [src/scenes/atlas-data.js](src/scenes/atlas-data.js) | 로컬 앱 '검증용 학습 자료 (Claude)' 워크스페이스의 과목 지도 스냅숏(개념 58·연결 76·단원 5) |
| [src/app-tokens.css](src/app-tokens.css) · [src/app-ui.css](src/app-ui.css) | 앱 밝은 테마 값(`app/ui/src/theme.rs`)과 재현 UI 스타일, 키캡 토스트, 포인터 |
| [src/style.css](src/style.css) · [src/sections.css](src/sections.css) | 어두운 페이지 스타일과 새 섹션 |
| [src/hero-scene.js](src/hero-scene.js) | 히어로 전체에 깔리는 S자 은하수와 별 하나(변경 없음) |
| [src/analytics.js](src/analytics.js) · [src/config.js](src/config.js) | 분석 래퍼와 배포 상수 |
| [public/media/](public/media/) · [public/fonts/](public/fonts/) | 실제 앱 이미지, 별 스프라이트, 공식 SUIT 가변 woff2와 [OFL](public/fonts/OFL.txt) |
| [scripts/check-catalog.mjs](scripts/check-catalog.mjs) | 정적 검사 |
| [qa/](qa/) | 브라우저 검증 스크립트와 화면 |
| [REFERENCE-NOTES.md](REFERENCE-NOTES.md) | 레퍼런스 관찰과 적용 원칙 |

한국어 문구는 `humanize-korean` 윤문을 한 번 거쳤다(2026-11-24, 변경률 0.1%). 런타임 UI 프레임워크 없이 Vite로 묶은 HTML·CSS·JavaScript다. 히어로와 06 과목 지도의 지도 부분은 Canvas 2D, 나머지 기능 시연은 DOM/CSS다. 외부 라이브러리·WebGL·영상 파일을 쓰지 않는다.

### 페이지 순서

히어로(은하수 고정, 신뢰 사실 네 개) → 01 한 화면(앱 창 전체 20초 루프: PDF 열기 → 형광펜 → AI 질문 ⌘↩ → 노트로 정리) → 02 지금은(중립 창 세 개가 하나로 합쳐지는 문제 제기) → 03 세 가지 약속 → 04 노트(핵심 6 · 모두 16) → 05 PDF 필기(핵심 5 · 모두 13) → 06 과목 지도(지식지도 조작 장면, 약 23초 반복) → 07 시작하기 세 단계 → 08 FAQ → 마무리 → 푸터(릴리스 노트·Discord·데이터 안내). 가격·요금제는 적지 않는다(사용자 결정).

### 기능 시연 장면

장면은 앱의 토큰(색·모서리·글꼴)으로 그린 프레임 위에서 입력 → 선택 → 키 → 결과 순서로 재생된다. 프레임은 설계 픽셀(문서·PDF·지도 720×450, 창 전체 1120×700)로 그린 뒤 `transform: scale`로 컨테이너 폭에 맞춰 글이 다시 흐르지 않는다. 키를 누르는 순간 프레임 하단 중앙의 어두운 알약 안에 키캡이 뜬다(자막 대신). 글리프 순서는 앱 문서 표기를 따른다. PDF 장면은 포인터가 도구를 고르고 움직이며, 키를 쓰는 순간(⌫)에만 토스트가 뜬다. 노트 장면은 9초, PDF 장면은 12초를 넘지 않으며 반복 사이에 0.9초 쉰다.

선택한 항목만 재생하고 화면 밖·백그라운드에서는 멈춘다. 시스템 동작 줄이기에서는 최종 상태와 키캡을 고정해 보여 준다. 수동 재생 조작은 두지 않는다. 장면 아래의 입력법·설명은 고정 텍스트다. 앱 UI 크롬(버튼·탭·상태 문구)은 두 언어 모두 한국어이고, 사용자 데이터(과목·파일 이름·본문·질문)만 페이지 언어를 따른다.

### 과목 지도 장면

06 섹션은 앱의 지식지도 화면(지도 카드 + 오른쪽 `개념과 원본 근거` 패널)을 재현해 조작 순서만 보여 준다. 설명 콜아웃은 없다. 빈 곳을 끌어 3D로 돌린다 → Cursor based 페이징을 열고 패널을 내려 배운 곳(강의자료 page 14)을 본다 → 더블클릭으로 전체 보기 → 다시 돌린다 → 개념 검색 `정규형` 입력·Enter(키캡) → 제1정규형의 배운 곳(내 질문에서) → 패널의 연결 문장으로 제2정규형.

- 데이터는 2026-11-24에 로컬 앱 데이터 사본에서 core `graph.get`(과목 층)으로 내보낸 스냅숏이다. 배치가 앱과 같도록 개념 id는 원본을 쓰고, 앱 경로·자료 id·`구버전` 표시는 뺐으며, 원문 인용은 장면이 여는 세 개념만 남겼다. 영어 페이지는 개념·단원·자료 이름과 장면에 보이는 설명·인용을 `en.js`의 번역표로 바꾼다. 관계 이름(선행·구성 등)과 앱 UI 문구는 한국어 그대로다.
- 지도는 앱과 같은 결정적 배치(FNV 해시 시드)·원근 카메라·감쇠 회전·720ms 이동·선택 시 자기중심 배치·강조·이름표 우선순위를 따른다. Metal 구체는 같은 조명식으로 칠한 스프라이트, 깊이는 먼 것부터 칠하는 근사다.
- 앱은 선택한 개념 주변을 지금 카메라 쪽으로 펼치므로 각도에 따라 가까운 구체가 이웃을 가린다. 장면은 개념을 열기 전 끄는 거리(`ATLAS_DRAGS`)를 [구도 검사](qa/atlas-composition.mjs)로 골랐다. 데이터나 순서를 바꾸면 `node landing/qa/atlas-composition.mjs --scan`으로 다시 고른다.
- 동작 줄이기에서는 마지막 상태(제2정규형 선택)와 Enter 키캡을 고정해 보여 준다.

### 한국어·영어

언어별로 URL을 나눈다(`/`, `/en/`). 헤더의 `KO · EN`은 다른 언어 페이지로 가는 링크이며 현재 해시를 유지한다. 자동 리디렉션은 하지 않는다. `VITE_SITE_URL`이 있을 때 두 페이지가 서로를 hreflang으로 가리키고 `x-default`는 한국어다. 영어 페이지는 요구 사항 줄과 FAQ에 인터페이스가 아직 한국어임을 밝힌다.

### 측정

`src/analytics.js`의 `track(name, props)`는 Umami가 없으면 아무 일도 하지 않는다. 이벤트: `download_click{place}`, `lang_switch{to}`, `scene_select{id, section}`, `scene_view_all{section, mode}`, `faq_open{id}`, `data_notice_open`, `outbound{place}`. 보조 지표로 GitHub 릴리스 자산의 `download_count`를 주 1회 적는다. 푸터의 데이터 안내가 사이트와 앱의 전송 범위를 한 문장씩 설명한다.

## 검사와 검증

`npm run check --prefix landing`은 항목 수(16·13), 핵심 수(6·5), id 중복, 장면 함수 존재, 한·영 문구 누락, FAQ의 가격 표현, 선택 구절이 본문에 있는지, 단축키가 앱 문서에 있는지, 지도 장면의 영어 번역표가 개념·단원·자료 이름과 장면 문구를 모두 덮는지, 지도 검색어의 첫 결과가 장면이 여는 개념인지를 검사한다. 영상 검사는 없다. 정적 검사 통과가 화면의 시각 검증을 대신하지 않는다.

브라우저 검증은 Aside REPL 스크립트 [qa/rebuild-check-desktop.js](qa/rebuild-check-desktop.js)(데스크톱 1440×900, 한국어)와 [qa/rebuild-check-mobile-en.js](qa/rebuild-check-mobile-en.js)(모바일 390, 영어, 동작 줄이기)로 하며, 결과는 [실행 기록](../app/qa/runs/landing-rebuild-2026-11-23.md)에 남긴다. 06 과목 지도 장면은 [qa/atlas-check.js](qa/atlas-check.js)(한국어·영어·동작 줄이기)와 [구도 검사](qa/atlas-composition.mjs)로 확인하고 [지도 장면 기록](../app/qa/runs/landing-atlas-2026-11-24.md)에 남겼다. 2026-11-22까지의 녹화 기반 검증 기록([실제 녹화 전환](../app/qa/runs/landing-recorded-2026-11-17.md) 등)과 히어로 검증([은하수 히어로](../app/qa/runs/landing-hero-galaxy-2026-11-22.md))은 이력이다. 히어로 코드는 이번 재구축에서 바꾸지 않았다.

## 결정 기록

정적 사이트 선택은 [ADR-023](../decisions/2026-11-17/ADR-023-korean-product-landing.md), 이전 모션·녹화 방식은 [ADR-024](../decisions/2026-11-17/ADR-024-feature-motion-catalog.md)·[ADR-025](../decisions/2026-11-17/ADR-025-passive-native-demos.md), 히어로는 [ADR-031](../decisions/2026-11-22/ADR-031-landing-hero-icon-ribbon-and-star-sprites.md)·[ADR-032](../decisions/2026-11-22/ADR-032-landing-hero-galaxy-stream-and-single-star.md)·[ADR-033](../decisions/2026-11-22/ADR-033-landing-hero-sky-behind-copy-and-free-star.md), 실제 UI 재현 장면·키캡 토스트·언어별 URL은 [ADR-039](../decisions/2026-11-23/ADR-039-landing-dom-scenes-key-toasts-and-locale-urls.md), 과목 지도의 Canvas 2D 조작 장면은 [ADR-040](../decisions/2026-11-24/ADR-040-landing-course-map-canvas-scene.md)에 있다. 녹화 자산의 처리는 [미디어 기록](public/media/README.md)을 본다.

랜딩 코드만 수정할 때는 Mac 앱 테스트·재설치가 필요하지 않다. 앱 코드를 함께 수정했다면 저장소 [작업 규칙](../AGENTS.md)을 따른다.
