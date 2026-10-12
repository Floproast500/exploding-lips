#!/bin/sh
# Full test run: unit/data tests, lints, WASM build, web smoke test and (with --original)
# a golden comparison against the original game under Wine.
set -eu
cd "$(dirname "$0")/.."
echo "== cargo test"
cargo test --workspace --all-features --quiet
echo "== clippy"
cargo clippy --workspace --all-targets --all-features --quiet -- -D warnings
echo "== release tools"
cargo build --release --quiet -p lipstool -p lips-desktop
if rustup target list --installed 2>/dev/null | grep -q x86_64-pc-windows-gnu && command -v x86_64-w64-mingw32-gcc >/dev/null; then
    echo "== windows cross-build"
    cargo build --release --quiet -p lips-desktop --target x86_64-pc-windows-gnu
fi
if rustup target list --installed 2>/dev/null | grep -q aarch64-apple-darwin; then
    echo "== macos check"
    cargo check --quiet -p lips-desktop --target aarch64-apple-darwin
fi
echo "== wasm"
./scripts/build-web.sh >/dev/null
if [ -d tools/pilot/node_modules ]; then
    echo "== web smoke test"
    node tools/pilot/web_pilot.mjs --level LIPS00 --out re/shots/web/lips00.png --keys ArrowLeft:400
fi
if [ "${1:-}" = "--original" ]; then
    echo "== golden comparison against lips.exe (Wine)"
    python3 -I tests/compare_original.py
fi
echo "all tests passed"
