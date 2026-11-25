#!/bin/bash
# 공개 배포본: Sparkle 포함 빌드 → Developer ID 서명 → DMG → 공증·staple → appcast.xml 갱신.
# Developer ID 인증서, notarytool 프로필(stology-notary), Sparkle 개인 키가 있는 배포용 Mac에서 실행한다.
set -euo pipefail
if [[ $# -ne 2 ]]; then
 echo "사용법: $0 <표시 버전 예: 0.1.0> <빌드 번호 예: 1>" >&2
 echo "빌드 번호는 공개할 때마다 이전보다 크게 올린다." >&2
 exit 1
fi
VERSION="$1"
BUILD="$2"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO="$(cd "$ROOT/.." && pwd)"
SPARKLE_DIR="${SPARKLE_DIR:-$HOME/Sparkle}"
PROFILE="${NOTARY_PROFILE:-stology-notary}"
DOWNLOAD_PREFIX="https://github.com/kjhh2605/Stology-App/releases/download/v$VERSION/"
APP="$ROOT/dist/Stology Local.app"
UPDATES="$ROOT/dist/updates"
DMG="$UPDATES/Stology-Local-$VERSION-arm64.dmg"
APPCAST="$REPO/landing/public/updates/appcast.xml"

if [[ ! -x "$SPARKLE_DIR/bin/generate_appcast" || ! -d "$SPARKLE_DIR/Sparkle.framework" ]]; then
 echo "Sparkle 배포 파일이 없습니다: $SPARKLE_DIR (SPARKLE_DIR로 위치 지정)" >&2
 exit 1
fi
xattr -dr com.apple.quarantine "$SPARKLE_DIR" 2>/dev/null || true
ID="$(security find-identity -v -p codesigning | grep -o '"Developer ID Application:[^"]*"' | head -1 | tr -d '"')"
if [[ -z "$ID" ]]; then
 echo "키체인에 Developer ID Application 인증서가 없습니다." >&2
 exit 1
fi
echo "서명 인증서: $ID"

STOLOGY_VERSION="$VERSION" STOLOGY_BUILD="$BUILD" STOLOGY_SPARKLE="$SPARKLE_DIR/Sparkle.framework" \
 "$ROOT/scripts/build.sh" --app-only

sign() { codesign --force --options runtime --timestamp --sign "$ID" "$@"; }
# 안쪽 코드부터 바깥 앱 순서로 서명한다. Sparkle 순서는 Sparkle 공식 안내를 따른다.
FW="$APP/Contents/Frameworks/Sparkle.framework"
sign "$FW/Versions/B/XPCServices/Installer.xpc"
sign --preserve-metadata=entitlements "$FW/Versions/B/XPCServices/Downloader.xpc"
sign "$FW/Versions/B/Autoupdate"
sign "$FW/Versions/B/Updater.app"
sign "$FW"
for binary in stology-core stology-extract stology-local; do sign "$APP/Contents/MacOS/$binary"; done
sign "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"

mkdir -p "$UPDATES"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
ditto "$APP" "$STAGE/Stology Local.app"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -volname 'Stology Local' -srcfolder "$STAGE" -format UDZO "$DMG"
codesign --sign "$ID" --timestamp "$DMG"

xcrun notarytool submit "$DMG" --keychain-profile "$PROFILE" --wait | tee "$STAGE/notary.log"
if ! grep -q 'status: Accepted' "$STAGE/notary.log"; then
 echo "공증이 통과하지 않았습니다. 위 id로 'xcrun notarytool log <id> --keychain-profile $PROFILE'를 확인하세요." >&2
 exit 1
fi
xcrun stapler staple "$DMG"
xcrun stapler validate "$DMG"
spctl -a -t open --context context:primary-signature -v "$DMG"

# 게시된 목록을 이어받아 이번 버전 항목을 추가한다.
if [[ -f "$APPCAST" ]]; then cp "$APPCAST" "$UPDATES/appcast.xml"; fi
"$SPARKLE_DIR/bin/generate_appcast" --download-url-prefix "$DOWNLOAD_PREFIX" "$UPDATES"
mkdir -p "$(dirname "$APPCAST")"
cp "$UPDATES/appcast.xml" "$APPCAST"
(cd "$UPDATES" && shasum -a 256 "$(basename "$DMG")" > SHA256SUMS)

cat <<DONE

공개 배포본 준비 완료
 DMG: $DMG
 appcast: $APPCAST

다음 순서로 게시한다. 목록(appcast)은 DMG를 올린 뒤에 게시한다.
 1. gh release create v$VERSION "$DMG" --title "Stology $VERSION"
 2. 브라우저로 ${DOWNLOAD_PREFIX}$(basename "$DMG") 다운로드 확인
 3. git add landing/public/updates/appcast.xml && git commit -m "Release $VERSION" && git push
DONE
