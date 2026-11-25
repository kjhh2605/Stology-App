# Stology Local

개인 자료를 Markdown으로 저장하고, 원본 자료를 구독 공급자로 분석해 개념·관계·근거를 탐색하는 Apple Silicon Mac 앱이다.

- 빌드 산출물: `dist/Stology Local.app`, 설치 이미지: `dist/Stology-Local-arm64.dmg`
- 제품 정책: [docs/product-policy.md](docs/product-policy.md)
- 단축키와 Markdown 입력: [docs/keyboard-shortcuts.md](docs/keyboard-shortcuts.md)
- 노트 이미지·다이어그램: [docs/note-images.md](docs/note-images.md)
- 외부 코드·라이선스: [docs/THIRD-PARTY.md](docs/THIRD-PARTY.md)

아래 빌드 명령은 저장소 루트 기준이며 `dist/`는 `app/dist/`를 뜻한다.

## 설치·첫 실행

1. `Stology-Local-arm64.dmg`를 열고 `Stology Local.app`을 Applications로 옮긴다.
2. 앱을 연다. ad-hoc 서명 빌드라 macOS가 실행을 막으면 앱 출처를 확인한 뒤 시스템 설정 → 개인정보 보호 및 보안에서 해당 앱의 열기 허용을 사용한다. Gatekeeper를 시스템 전체에서 해제하지 않는다.
3. 과목을 만들고 중앙에서 작성한다. 첫 저장은 제목·폴더를 선택한다. 저장 전 초안은 작성 중에 보관된다. 자료 폴더 우클릭 → 새 노트는 그 폴더에 즉시 파일을 만들고 중앙에서 작성하며, 빈 파일도 유지한다.
4. 구독 계정 연결에서 Codex 또는 Claude를 선택한다. 공식 CLI가 없다면 선택한 공급자의 설치 절차를 진행한다. 브라우저 인증은 공식 계정 화면에서 수행한다.
5. 지식지도 펼치기 → 자료 추가하기로 파일을 선택한다. **추출 텍스트와 필요한 이미지가 선택한 구독 공급자에게 전송된다.** 원본은 분석 실패 때도 보존한다.

설치·인증·실제 분석 성공은 별도 상태다. 계정·구독 한도·네트워크·공급자 제품 내 사용 조건에 따라 분석이 제한될 수 있다. 구독 실패를 API 과금으로 바꾸지 않는다. Claude는 변경하지 않은 공식 도구를 실행하고 사용자가 공식 인증으로 자신의 구독을 사용한다. 자체 Claude.ai 로그인 화면이나 토큰 수집·저장·중계로 확장하지 않는다.

## 재현 빌드와 검사

Apple Silicon Mac, Xcode Command Line Tools, Rust/Cargo가 필요하다. 공식 AI CLI는 기본 앱에 내장하지 않는다.

```sh
./app/scripts/build.sh
./app/scripts/test.sh
```

이 Mac의 사용 앱까지 갱신하려면 앱을 종료한 뒤 `./app/scripts/build.sh --install`을 실행한다.
앱만 빌드·설치하려면 `--app-only --install`을 함께 사용한다. 설치본을 서명 검증 후 교체하고
이전 앱은 따로 보관하지 않는다(교체 실패 시에만 임시 사본으로 복구). 문서 저장소는 변경하지 않는다.
평소에는 Dock 또는 `/Applications/Stology Local.app`을 사용한다. `dist`는 패키징 산출물이다.

3D 지도의 그리기 품질과 모션은 `app/ui`의 예제로 따로 확인한다: `graph3d-smoke`(실제 GPU 깊이·절단), `graph-quality`(구 윤곽 편차·관계선 폭·장면 캡처), `graph-live`(실제 창의 모션 루프와 포인터·스크롤 처리, 작은 창이 15초쯤 뜬다).

앱만 만들려면 `./app/scripts/build.sh --app-only`를 사용한다. `Cargo.lock`을 사용하는 release 빌드와 `swiftc -O` 시스템 추출 헬퍼를 묶고 서명을 검증한다. DMG 해시는 `dist/SHA256SUMS`에 기록한다.

## 데이터와 복구

기본 데이터는 `~/Library/Application Support/Stology Local/`에 있다. `workspaces/<과목 ID>/files/`에 실제 MD·원본, `storage.sqlite`에 문서 메타데이터·초안, `versions/`에 분석용 불변 원본을 보관한다. 그래프와 작업 기록은 같은 앱 전용 저장소의 별도 내부 데이터다. 원본과 SQLite를 함께 보존하려면 앱과 분석을 종료한 뒤 저장소 전체를 백업한다. 내보내기 UI가 있다는 뜻은 아니다.

PPTX 시스템 미리보기가 내용을 표시하지 못하면 이미 설치된 LibreOffice로 내부 PDF 변환 미리보기를 사용할 수 있다. 변환기는 앱에 포함하거나 자동 설치하지 않는다. 없거나 변환이 실패하면 PDF 내보내기 입력을 안내한다. 원본 PPTX는 보존한다.

개발·QA에서는 `STOLOGY_DATA_DIR` 환경변수로 데이터 위치를 분리할 수 있다. 삭제 시험에 개인 원본을 쓰지 않는다. 삭제 대상은 확인 후 휴지통으로 이동한다. 분석된 MD를 편집해도 과거 근거 버전은 따로 보존하며 재분석은 명시적으로 요청한다.

## 검증 범위

확정 정책·체크리스트 작성, 자동 테스트, 직접 UI 조작, 실제 구독 분석은 서로 다른 증거다. 실제 로그인·이미지 분석·한글 조합·최종 설치 앱 수용 흐름을 관찰하지 못한 항목은 완료로 간주하지 않는다.

외부 코드·의존성 라이선스는 [docs/THIRD-PARTY.md](docs/THIRD-PARTY.md)에 기록한다. Tolaria 코드는 복사하지 않는다.

노트 상단의 다이어그램 버튼으로 도형과 화살표를 그리고 본문에 삽입합니다. 그림을 더블클릭하면 다시 편집할 수 있습니다. [다이어그램 사용법](docs/note-images.md#노트-다이어그램)
