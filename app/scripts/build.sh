#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="$ROOT/dist"
APP="$DIST/Stology Local.app"
APP_ONLY=false
INSTALL=false
for option in "$@"; do
 case "$option" in
  --app-only) APP_ONLY=true ;;
  --install) INSTALL=true ;;
  *) echo "사용법: $0 [--app-only] [--install]" >&2; exit 1 ;;
 esac
done
if [[ "$INSTALL" == true ]]; then
 python3 "$ROOT/scripts/install.py" --check-idle
fi
# 공개 배포(scripts/release.sh)가 지정한다. 개발 빌드는 기본값과 Sparkle 없는 앱을 쓴다.
VERSION="${STOLOGY_VERSION:-0.1.0}"
BUILD="${STOLOGY_BUILD:-1}"
SPARKLE="${STOLOGY_SPARKLE:-}"
FEED_URL='https://stology-app.vercel.app/updates/appcast.xml'
SPARKLE_PUBLIC_KEY='pWi1bdG0ZNvvkjO7LTCdSwWH7gia/gQ/Nuo0EX+uJtg='
export MACOSX_DEPLOYMENT_TARGET=14.0
if [[ "$(uname -m)" != arm64 ]]; then echo 'Apple Silicon Mac이 필요합니다.' >&2; exit 1; fi
cargo build --locked --release --manifest-path "$ROOT/core/Cargo.toml"
cargo build --locked --release --manifest-path "$ROOT/ui/Cargo.toml"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ROOT/assets/StologyLocal.icns" "$APP/Contents/Resources/"
mkdir -p "$APP/Contents/Resources/fonts"
cp "$ROOT/assets/fonts/"*.otf "$APP/Contents/Resources/fonts/"
cp "$ROOT/core/target/release/stology-core" "$APP/Contents/MacOS/"
cp "$ROOT/ui/target/release/stology-local" "$APP/Contents/MacOS/"
xcrun swiftc -O -target arm64-apple-macosx14.0 "$ROOT/native/Extraction.swift" -o "$APP/Contents/MacOS/stology-extract" -framework PDFKit -framework Vision -framework AppKit -framework ImageIO
cp -R "$ROOT/core/ai" "$APP/Contents/Resources/"
python3 "$ROOT/scripts/third-party.py"
mkdir -p "$APP/Contents/Resources/Licenses"
cp "$ROOT/docs/THIRD-PARTY.md" "$APP/Contents/Resources/Licenses/"
cp "$ROOT/assets/fonts/OFL.txt" "$APP/Contents/Resources/Licenses/SUIT-OFL.txt"
cp -R "$ROOT/docs/third-party" "$APP/Contents/Resources/Licenses/"
rm -rf "$APP/Contents/Frameworks"
SPARKLE_KEYS=''
if [[ -n "$SPARKLE" ]]; then
 mkdir -p "$APP/Contents/Frameworks"
 ditto "$SPARKLE" "$APP/Contents/Frameworks/Sparkle.framework"
 SPARKLE_KEYS="<key>SUFeedURL</key><string>$FEED_URL</string>
<key>SUPublicEDKey</key><string>$SPARKLE_PUBLIC_KEY</string>"
fi
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Stology Local</string>
<key>CFBundleDevelopmentRegion</key><string>ko</string>
<key>CFBundleLocalizations</key><array><string>ko</string></array>
<key>CFBundleDisplayName</key><string>Stology Local</string>
<key>CFBundleIdentifier</key><string>app.stology.local</string>
<key>CFBundleVersion</key><string>$BUILD</string>
<key>CFBundleShortVersionString</key><string>$VERSION</string>
<key>CFBundleExecutable</key><string>stology-local</string>
<key>CFBundleIconFile</key><string>StologyLocal</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>LSMinimumSystemVersion</key><string>14.0</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSPrincipalClass</key><string>NSApplication</string>
<key>NSHumanReadableCopyright</key><string>Stology Local — personal test build</string>
$SPARKLE_KEYS
</dict></plist>
PLIST
if [[ -n "$SPARKLE" ]]; then codesign --force --deep --sign - "$APP/Contents/Frameworks/Sparkle.framework"; fi
for binary in stology-core stology-extract stology-local; do codesign --force --sign - "$APP/Contents/MacOS/$binary"; done
codesign --force --sign - "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"
if [[ "$APP_ONLY" != true ]]; then
 STAGE="$(mktemp -d "$DIST/dmg-stage.XXXXXX")"
 trap 'rm -rf "$STAGE"' EXIT
 cp -R "$APP" "$STAGE/"
 ln -s /Applications "$STAGE/Applications"
 hdiutil create -volname 'Stology Local' -srcfolder "$STAGE" -ov -format UDZO "$DIST/Stology-Local-arm64.dmg"
 (cd "$DIST" && shasum -a 256 Stology-Local-arm64.dmg > SHA256SUMS)
fi
if [[ "$INSTALL" == true ]]; then
 python3 "$ROOT/scripts/install.py"
fi
printf '개인 Mac 테스트용 ad-hoc 서명 빌드: %s\nApple 공증 완료 배포본이 아닙니다.\n' "$APP"
