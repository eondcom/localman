#!/bin/bash
# LocalMan.app 을 만들어 /Applications 에 설치한다 (맥).
#   ./scripts/macos/make-app.sh            # 빌드 + 설치
#   ./scripts/macos/make-app.sh --no-install   # target/LocalMan.app 만 만든다
set -e
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:$PATH"

VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
# LOCALMAN_BINARY 를 주면 그 실행 파일을 쓴다 (release.sh 의 universal 빌드)
BINARY=${LOCALMAN_BINARY:-target/release/localman}
[ -n "$LOCALMAN_BINARY" ] || cargo build --release

APP=target/LocalMan.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BINARY" "$APP/Contents/MacOS/LocalMan"

# 아이콘: assets/localman.png → .icns
ICONSET=target/localman.iconset
rm -rf "$ICONSET"; mkdir -p "$ICONSET"
for s in 16 32 128 256; do
  sips -z $s $s assets/localman.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z $d $d assets/localman.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/LocalMan.icns"
# Homebrew 등으로 앱만 받은 경우에도 쓸 수 있게 sudoers 설치 스크립트를 앱 안에 넣는다
cp scripts/macos/install-sudoers.sh "$APP/Contents/Resources/install-sudoers.sh"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>LocalMan</string>
  <key>CFBundleDisplayName</key><string>LocalMan</string>
  <key>CFBundleIdentifier</key><string>com.eond.localman</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleExecutable</key><string>LocalMan</string>
  <key>CFBundleIconFile</key><string>LocalMan</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# 서명 없는 앱은 실행이 막힐 수 있어 이 PC 용 임시 서명(ad-hoc)을 붙인다
codesign --force --deep -s - "$APP"

if [ "$1" != "--no-install" ]; then
  rm -rf /Applications/LocalMan.app
  cp -R "$APP" /Applications/
  echo "설치 완료: /Applications/LocalMan.app (v$VERSION)"
else
  echo "만듦: $APP (v$VERSION)"
fi
