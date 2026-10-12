#!/bin/sh
# Build the WebAssembly front end into crates/lips-web/www (serve that directory over HTTP).
set -eu
cd "$(dirname "$0")/.."
PATH="$PATH:${CARGO_HOME:-$HOME/.cargo}/bin"
cargo build -p lips-web --target wasm32-unknown-unknown --release
wasm-bindgen --target web --no-typescript --out-dir crates/lips-web/www/pkg \
    target/wasm32-unknown-unknown/release/lips_web.wasm
echo "built crates/lips-web/www — try: python3 -m http.server -d crates/lips-web/www 8080"
