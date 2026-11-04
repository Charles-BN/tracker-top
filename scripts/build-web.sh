#!/usr/bin/env bash
# Build the browser version of tracker-top into target/web-dist/.
#
# Needs: the Rust target wasm32-unknown-unknown and wasm-bindgen-cli at the same
# version as the wasm-bindgen crate (see crates/top-web/Cargo.toml):
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version 0.2.130 --locked
#
# The result is a folder of static files: serve it from any web server
# (GitHub Pages, an intranet server...). It does not work opened from disk
# (file://), because browsers refuse WebAssembly workers there.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=target/web-dist

cargo build --release --target wasm32-unknown-unknown -p top-web
rm -rf "$OUT"
mkdir -p "$OUT/pkg"
wasm-bindgen --target web --no-typescript --out-dir "$OUT/pkg" \
  target/wasm32-unknown-unknown/release/top_web.wasm

cp -r crates/top-app/web/. "$OUT/"
cp crates/top-web/www/*.js "$OUT/"
# Switch the page to the in-browser engine.
sed -i 's#<script type="module" src="app.js"></script>#<script>window.TRACKER_TOP_BROWSER = true;</script>\n<script type="module" src="app.js"></script>#' "$OUT/index.html"
grep -q TRACKER_TOP_BROWSER "$OUT/index.html"
# Stop GitHub Pages from ignoring files that start with an underscore.
touch "$OUT/.nojekyll"
echo "Browser version built in $OUT ($(du -sh "$OUT" | cut -f1))"
