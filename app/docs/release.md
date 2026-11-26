# 업데이트 배포 절차

Stology Local은 앱 안의 Sparkle이 웹에 게시된 `appcast.xml`을 읽어 새 버전을 받습니다. 새 버전은 `app/scripts/release.sh`로 빌드·서명·공증하고 DMG는 GitHub Releases, 목록은 Vercel에 게시합니다. 개발용 설치(`build.sh --app-only --install`)와는 별개의 공개 배포 절차입니다.

## 배포용 Mac 준비 (처음 한 번)

| 항목 | 용도 | 확인 명령 |
|---|---|---|
| Developer ID Application 인증서(개인 키 포함) | 앱·DMG 서명 | `security find-identity -v -p codesigning \| grep "Developer ID Application"` |
| notarytool 프로필 `stology-notary` | Apple 공증 | `xcrun notarytool history --keychain-profile stology-notary` |
| `~/Sparkle` 폴더와 EdDSA 개인 키 | appcast 생성·업데이트 서명 | `~/Sparkle/bin/generate_keys -p` |

- 인증서를 `.p12`로 옮겼는데 "유효한 ID"로 안 잡히면 중간 인증서(Developer ID Certification Authority G2)가 없는 것이다. Apple 공식 주소 `https://www.apple.com/certificateauthority/DeveloperIDG2CA.cer`를 받아 로그인 키체인에 추가한다.
- 공증 프로필은 옮기지 않고 새 Mac에서 앱 암호로 다시 저장한다. 앱 암호는 만들 때 한 번만 보이므로 필요하면 account.apple.com에서 새로 만든다. 대화형 입력이 필요하므로 Terminal 앱에서 실행한다.
  `xcrun notarytool store-credentials stology-notary --apple-id <Apple 계정 이메일> --team-id B8Q8XMT6LP`
- 개인 키는 기존 배포 Mac에서 `generate_keys -x 파일`로 내보내 AirDrop·USB로만 옮기고, 새 Mac에서 `generate_keys -f 파일`로 가져온 뒤 파일을 지운다.
- **`generate_keys`를 옵션 없이 실행하지 않는다.** 새 키가 만들어지고, 그 키로 서명한 업데이트는 기존 사용자의 앱이 거부한다.
- `generate_keys -p`의 공개 키는 `app/scripts/build.sh`의 `SPARKLE_PUBLIC_KEY`(`pWi1bdG0ZNvvkjO7LTCdSwWH7gia/gQ/Nuo0EX+uJtg=`)와 같아야 한다.

## 버전마다 반복

예시는 `0.1.2`, 빌드 번호 `3`이다. 빌드 번호는 공개할 때마다 이전보다 크게 올린다(Sparkle은 빌드 번호로 새 버전을 판단한다).

1. 배포 전: 넣을 변경을 커밋·푸시하고 `cargo test --manifest-path app/core/Cargo.toml`, `cargo test --manifest-path app/ui/Cargo.toml`을 통과시킨다. `release.sh`는 작업 트리를 그대로 빌드한다.
2. 빌드·서명·공증·appcast 생성(몇 분): `./app/scripts/release.sh 0.1.2 3`
3. DMG를 먼저 올린다: `gh release create v0.1.2 app/dist/updates/Stology-Local-0.1.2-arm64.dmg --title "Stology 0.1.2" --notes "Stology Local 0.1.2" --prerelease` (정식이면 `--prerelease` 제외)
4. 다운로드 확인: `curl -sIL -o /dev/null -w "%{http_code}\n" https://github.com/kjhh2605/Stology-App/releases/download/v0.1.2/Stology-Local-0.1.2-arm64.dmg` → `200`
5. 랜딩 버전과 appcast 게시: `landing/src/config.js`의 `DOWNLOAD_VERSION`을 새 버전으로 바꾸고 `landing/public/updates/appcast.xml`과 함께 커밋·푸시한다.
6. 게시 확인: `curl -s https://stology-app.vercel.app/updates/appcast.xml | grep -o "<sparkle:shortVersionString>[^<]*"` (Vercel 반영에 1분 정도)

**순서를 지킨다.** DMG 업로드(3·4) 뒤에 appcast를 게시(5)해야 한다. 순서가 바뀌면 사용자가 아직 없는 파일을 받으려 한다. DMG 다운로드 주소가 로그인하지 않은 브라우저에서 열려야 하므로 저장소는 공개 상태여야 한다.

## 배포 후 확인

- GitHub Releases에 `v버전` 태그와 DMG가 있다.
- 공개 appcast에 새 버전 항목이 보인다.
- 이전 버전 앱의 앱 메뉴 **업데이트 확인…**에서 "새 버전이 있습니다"가 뜨고, 설치 후 새 버전으로 재실행된다. 2026-11-25 0.1.0 → 0.1.1에서 이 경로(다운로드·서명 검증·설치·재실행, 공증 통과)를 확인했다.
- 랜딩 페이지 다운로드 버튼이 새 버전을 가리킨다.

## 문제가 생기면

| 증상 | 확인할 것 |
|---|---|
| 인증서가 있는데 유효하지 않음 | 위의 중간 인증서 설치 |
| 공증이 통과하지 않음 | 출력된 id로 `xcrun notarytool log <id> --keychain-profile stology-notary` |
| 앱이 업데이트를 거부함 | 공개 키가 `build.sh`와 같은지, 빌드 번호를 올렸는지 |
| 다운로드가 404 | 저장소 공개 여부, 릴리스 태그와 DMG 파일 이름 |
| appcast가 404 | `landing/public/updates/appcast.xml` 커밋·푸시 여부와 Vercel 배포 상태 |

참고: [Sparkle · Publishing an update](https://sparkle-project.org/documentation/publishing/), [Apple · Customizing the notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)
