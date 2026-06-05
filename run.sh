#!/bin/bash
# kime와 충돌 방지: winit를 XWayland로 강제
export WINIT_UNIX_BACKEND=x11
cargo run --manifest-path "$(dirname "$0")/Cargo.toml" "$@"
