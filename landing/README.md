# Stology 한국어 소개 사이트

처음 사용하는 대학생을 위한 독립 정적 랜딩사이트다. 현재 Mac 앱의 노트, PDF 필기, 지식지도, 학습도우미를 소개한다. 노트 입력·단축키와 PDF 기능은 실제 Mac 앱에서 촬영한 영상으로 설명한다. 제품 범위는 [로컬앱 제품 정책](../app/docs/product-policy.md), 노트 기능은 [앱 단축키 문서](../app/docs/keyboard-shortcuts.md)를 기준으로 한다.

## 로컬 실행

Node.js와 npm이 필요하다. 아래 명령은 저장소 루트 기준이다.

```sh
npm install --prefix landing
npm run dev --prefix landing
```

브라우저에서 `http://127.0.0.1:5173`을 연다. 해당 포트를 이미 사용 중이면 Vite가 출력한 주소를 따른다. 잠금 파일이 있는 환경의 재현 설치에는 `npm ci --prefix landing`을 사용할 수 있다.

```sh
npm run build --prefix landing
npm run preview --prefix landing
```

빌드 결과는 `landing/dist/`다. 정적 호스팅 서비스의 사이트 루트에 이 디렉터리를 배포할 수 있다. 현재 코드의 자산 URL은 `/` 기준이므로 하위 경로 배포에는 자산 경로 조정이 필요하다. 공개 도메인과 호스팅 배포는 아직 수행하지 않았다.

## 구조와 동작

| 경로 | 역할 |
|---|---|
| [index.html](index.html) | 한국어 문서·검색 메타 정보 |
| [src/main.js](src/main.js) | 페이지, FAQ, 사용 환경 안내 |
| [src/catalog.js](src/catalog.js) | 노트·PDF 기능 목록, 설명·단축키·Markdown 입력법·근거 위치 |
| [src/feature-explorer.js](src/feature-explorer.js) | 핵심 기능 검색·분류·선택과 영상 아래 입력법·설명 |
| [src/video-player.js](src/video-player.js) | 조작 UI 없는 실제 녹화 영상의 자동 반복·가시성 관리 |
| [src/copy.json](src/copy.json) | 한국어 주요 설명 문구 |
| [src/style.css](src/style.css) | 데스크톱·모바일 화면과 동작 줄이기 대응 |
| [src/hero-scene.js](src/hero-scene.js) | 히어로 전체에 깔리는 하늘, S자로 모여 흐르는 은하수, 그 위에서 자유롭게 움직이는 라벤더 별 하나의 Canvas 2D 메인 모션 |
| [public/media/](public/media/) | 실제 앱 화면 이미지, 기능별 MP4·JPG, 촬영 기록 |
| [public/media/hero/](public/media/hero/) · [public/mark.png](public/mark.png) | Blender로 렌더한 라벤더 별 스프라이트, 앱 아이콘에서 분리한 S 마크 |
| [scripts/encode-recordings.py](scripts/encode-recordings.py) | 완료된 원본 녹화의 H.264 인코딩·포스터·메타데이터 생성 |
| [scripts/extract-icon-s.py](scripts/extract-icon-s.py) | 앱 아이콘 PNG에서 S 마크 분리 |
| [scripts/render-star-sprites.py](scripts/render-star-sprites.py) · [scripts/build-star-sheet.py](scripts/build-star-sheet.py) | 라벤더 별 Blender 원본의 회전 프레임 렌더와 스프라이트 시트 생성 |
| [public/fonts/](public/fonts/) | 자체 제공 SUIT 폰트와 [OFL 라이선스](public/fonts/OFL.txt) |
| [REFERENCE-NOTES.md](REFERENCE-NOTES.md) | 모션 레퍼런스 관찰과 고유 디자인 적용 원칙 |
| [qa/coverage.json](qa/coverage.json) | 기능 목록과 현재 문서·구현 근거의 대응 |
| [qa/](qa/) | 랜딩사이트 브라우저 검증 자료 |

런타임 UI 프레임워크 없이 Vite로 묶은 HTML·CSS·JavaScript를 제공한다. 메인 장면은 Canvas 2D로 유지하며 라이브러리나 WebGL을 쓰지 않는다. 캔버스는 히어로 전체 폭에 깔리고 글과 버튼이 그 위에 놓인다. 은하수는 오른쪽 위에서 들어와 앱 아이콘의 S를 이루고 글 뒤를 지나 왼쪽으로 빠져나간다. 라벤더 별 하나가 흐름을 타고, 뛰어오르고, 다른 자리로 건너뛰거나 멀리 돌거나 한 자리에 머무는 행동을 방문할 때마다 다른 순서로 고른다. 포인터 아래의 별들은 갈라지며 휘돌고, 은하수를 누르면 고리가 퍼지며, 별을 누르면 뛰어오르며 한 바퀴 돈다. 머리글·바닥글의 마크는 앱 아이콘에서 분리한 S다. 기능 설명은 선택한 항목의 실제 녹화 영상을 `autoplay`, `muted`, `loop`, `playsinline`으로 재생한다. 영상에는 재생·일시정지 버튼, 탐색 막대, 전체 화면·화면 속 화면·원격 재생 조작, 컨텍스트 메뉴를 제공하지 않는다. 영상 아래의 입력법과 기능 설명은 일반 텍스트이며 영상 자막이 아니다. 화면 밖이나 백그라운드 탭에서는 불필요한 재생을 멈추고 다시 보일 때 이어 재생한다.

메인 장면의 수동 정지와 시스템 동작 줄이기 대응은 별도로 유지한다. 동작 줄이기에서는 정해 둔 한 장면을 정지 화면으로 보여 준다. 이전 `motion-player.js`, `note-scenes.js`, `note-special-scenes.js`, `pdf-scenes.js`는 현재 페이지에서 가져오지 않는 이전 구현이다.

## 설명 범위

현재 카탈로그는 노트 **16개**, PDF **13개** 핵심 항목을 제공한다. 노트는 글자 서식·제목·목록·인용·선택 확장·지도 검색을, PDF는 필기 도구·주석 편집·공유·변환·분석을 소개한다. 화살표 변환, Enter로 인용 나가기 같은 세부 입력 동작과 실행 취소·복사·붙여넣기 등 기본 명령은 사용자 요청에 따라 제외했다. 두 목록의 번호는 각각 1부터 연속으로 매긴다. 목록 검색과 분류로 기능을 찾고 항목을 선택하면 우측 영상과 하단 설명이 함께 바뀐다. [대응 기록](qa/coverage.json)에서 선별 범위와 항목별 근거를 확인할 수 있다. 항목 수는 앱 전체 기능의 수나 실제 앱 실기 검증 건수를 뜻하지 않는다.

단축키와 같은 기능을 만드는 Markdown 입력을 같은 항목에서 함께 표시한다. 제목 1·2·3에는 각각 `#`·`##`·`###` 다음 `Space`를, 코드 블록에는 백틱 세 개·언어 이름·`Enter`와 여닫는 백틱 표기를 보여 준다. 기본 명령이나 부가 입력 동작을 별도 항목으로 되살리지는 않는다.

시연 자료는 교양 수업인 ‘심리학의 이해’의 기억·학습 예시로 작성했다. 개인 데이터와 분리한 시험용 Stology에서 실제 조작을 촬영하고, 관련 부분의 크롭·축소와 영상 인코딩을 거쳐 제공한다. 설명용 DOM 장면을 녹화한 영상으로 대체하지 않는다. 녹화 도구와 자산 정보는 [촬영 기록](public/media/README.md), [영상 manifest](public/media/manifest.json)에 남긴다. 영상에는 음성·자막·VTT 트랙을 넣지 않는다. PDF의 저장 대상은 하단 안내에서 확인할 수 있다.

- 주요 설명 문구는 사용자가 지정한 `im-not-ai`의 `humanize-korean` 절차로 윤문했다.
- 다운로드 버튼(헤더·히어로·마무리·사용 환경 안내)은 GitHub 릴리스의 공증된 DMG로 바로 연결한다. 새 버전을 게시하면 `src/main.js`의 `DOWNLOAD_VERSION`을 올린다.
- 계정, 서버, 수집 폼, 분석 추적기를 추가하지 않았다. AI 연결·자료 전송 안내는 현재 앱의 제품 정책을 따른다.

랜딩 코드만 수정할 때는 Mac 앱 테스트·재설치가 필요하지 않다. 앱 코드를 별도로 수정했다면 저장소 [작업 규칙](../AGENTS.md)을 따른다. 정적 사이트 선택은 [ADR-023](../decisions/2026-11-17/ADR-023-korean-product-landing.md), 이전 모션 구현은 [ADR-024](../decisions/2026-11-17/ADR-024-feature-motion-catalog.md), 현재 실제 녹화·자동 반복 방식은 [ADR-025](../decisions/2026-11-17/ADR-025-passive-native-demos.md), 마크와 별 스프라이트는 [ADR-031](../decisions/2026-11-22/ADR-031-landing-hero-icon-ribbon-and-star-sprites.md), S자 은하수와 별 하나는 [ADR-032](../decisions/2026-11-22/ADR-032-landing-hero-galaxy-stream-and-single-star.md), 전체 폭 하늘·포인터 반응·별의 무작위 행동은 [ADR-033](../decisions/2026-11-22/ADR-033-landing-hero-sky-behind-copy-and-free-star.md)에 기록했다.

## 검증 기록 해석

[최초 랜딩 검증](../app/qa/runs/landing-site-2026-11-17.md)은 실제 녹화 7개를 사용하던 버전의 결과다. [전체 기능 모션 검증](../app/qa/runs/landing-motion-2026-11-17.md)은 노트 82개·PDF 25개를 소개하던 이전 버전의 결과다. 두 기록은 당시의 검증 이력이며 현재 선별 목록의 검증 결과로 해석하지 않는다. [기능 대응 기록](qa/coverage.json)은 문서·구현과 현재 카탈로그의 대응을 나타내며, 브라우저 검증과 실제 앱 동작 검증은 별도로 확인한다.

[핵심 기능 선별 검증](../app/qa/runs/landing-curated-2026-11-17.md)은 16개·13개로 목록을 줄였을 때의 모션 버전 이력이다. 현재 녹화 버전의 재생·화면 검증과 앱 기울임 수정 검증은 [실제 녹화 전환 검증](../app/qa/runs/landing-recorded-2026-11-17.md)에서 확인한다. 현재 히어로 장면은 [은하수 히어로 검증](../app/qa/runs/landing-hero-galaxy-2026-11-22.md)과 [브라우저 검사](qa/hero-scene-check.js)에서 확인한다. [이전 히어로 검증](../app/qa/runs/landing-hero-stars-2026-11-22.md)은 S 이미지와 별 여러 개를 쓰던 버전의 이력이다.

`npm run check --prefix landing`은 선별한 항목의 순서·근거·Markdown 표기, MP4·포스터 존재, H.264 단일 영상 스트림, 자막·조작 UI 부재를 검사한다. 영상 검사에는 `ffprobe`가 필요하다. 원문 단축키 표 전체를 포함하도록 요구하지 않으며, 정적 검사 통과가 영상 내용의 시각 검증을 대신하지 않는다.
