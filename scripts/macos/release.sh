#!/bin/bash
# 맥 배포용 LocalMan.dmg 를 만든다 — Intel·Apple Silicon 겸용(universal).
#   ./scripts/macos/release.sh            → target/release-dmg/LocalMan-<버전>.dmg
# GitHub 릴리스에 올린 뒤 eondcom/homebrew-tap 의 Casks/localman.rb 버전·sha256 을 바꾼다.
set -e
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:$PATH"
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)

rustup target add x86_64-apple-darwin aarch64-apple-darwin >/dev/null
cargo build --release --target x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
mkdir -p target/universal
lipo -create -output target/universal/localman \
  target/x86_64-apple-darwin/release/localman \
  target/aarch64-apple-darwin/release/localman
lipo -info target/universal/localman

# 앱 묶음은 make-app.sh 와 같은 방식 (실행 파일만 universal 로 바꿔 넣는다)
LOCALMAN_BINARY=target/universal/localman ./scripts/macos/make-app.sh --no-install

OUT=target/release-dmg
rm -rf "$OUT" && mkdir -p "$OUT/stage"
cp -R target/LocalMan.app "$OUT/stage/"
ln -s /Applications "$OUT/stage/Applications"
hdiutil create -volname "LocalMan" -srcfolder "$OUT/stage" -ov -format UDZO "$OUT/LocalMan-$VERSION.dmg" >/dev/null
rm -rf "$OUT/stage"
shasum -a 256 "$OUT/LocalMan-$VERSION.dmg"
