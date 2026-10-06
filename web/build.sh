#!/bin/sh
# Build the browser version into web/dist: a static site (index.html, the wasm and its JS glue)
# that any web server can host. Needs the wasm target and the wasm-bindgen CLI at the version in
# Cargo.lock:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version <the wasm-bindgen version in Cargo.lock>
# Serve it locally with: web/build.sh && python3 -m http.server -d web/dist 8080
set -e
cd "$(dirname "$0")/.."

cargo build --release --target wasm32-unknown-unknown
rm -rf web/dist
mkdir -p web/dist
wasm-bindgen --target web --no-typescript --out-dir web/dist --out-name tripslop \
	target/wasm32-unknown-unknown/release/tripslop.wasm
# Optional: shrink it further if binaryen is installed.
if command -v wasm-opt >/dev/null; then
	wasm-opt -O2 --enable-bulk-memory --enable-nontrapping-float-to-int -o web/dist/tripslop_bg.wasm web/dist/tripslop_bg.wasm
fi
cp web/index.html web/dist/
cp logos/tripslop-flower-color.svg web/dist/favicon.svg
cp logos/tripslop-wordmark-cream.svg web/dist/wordmark.svg
ls -lh web/dist
