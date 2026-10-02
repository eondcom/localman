#!/bin/bash
# 맥에서 리눅스 대상 타입 검사 (실행 파일은 만들지 않는다).
# ring 의 C 코드는 리눅스 헤더가 없어 맥 SDK 헤더로 대신 컴파일한다 — 검사 전용.
set -e
# Homebrew rust 가 PATH 앞에 있으면 리눅스 표준 라이브러리가 없다 — rustup 툴체인을 쓴다
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."
rustup target add x86_64-unknown-linux-gnu >/dev/null 2>&1 || true
SDK=$(xcrun --show-sdk-path)
CC_x86_64_unknown_linux_gnu=clang \
CFLAGS_x86_64_unknown_linux_gnu="--target=x86_64-unknown-linux-gnu -isystem $SDK/usr/include" \
  cargo check --target x86_64-unknown-linux-gnu --all-targets "$@"
