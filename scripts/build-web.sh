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

# Version stamp on every file address, so that browsers and company proxies
# never mix the files of two versions (they may keep old copies for a while),
# and version.json, which the page checks to offer a reload after an update.
BUILD="$(git rev-parse --short=10 HEAD 2>/dev/null || date +%s)-$(date -u +%Y%m%d%H%M%S)"
sed -i -e "s#href=\"style.css\"#href=\"style.css?v=$BUILD\"#" \
       -e "s#src=\"config.js\"#src=\"config.js?v=$BUILD\"#" \
       -e "s#src=\"app.js\"#src=\"app.js?v=$BUILD\"#" \
       -e "s#window.TRACKER_TOP_BROWSER = true;#window.TRACKER_TOP_BROWSER = true; window.TRACKER_TOP_BUILD = '$BUILD';#" \
       "$OUT/index.html"
sed -i -e "s#'\./profile.js'#'./profile.js?v=$BUILD'#" -e "s#'\./sync.js'#'./sync.js?v=$BUILD'#" \
       -e "s#'\./view3d.js'#'./view3d.js?v=$BUILD'#" -e "s#'\./browser.js'#'./browser.js?v=$BUILD'#" "$OUT/app.js"
sed -i "s#'\./worker.js'#'./worker.js?v=$BUILD'#" "$OUT/browser.js"
sed -i "s#'\./pkg/top_web.js'#'./pkg/top_web.js?v=$BUILD'#" "$OUT/worker.js"
sed -i "s#'top_web_bg.wasm'#'top_web_bg.wasm?v=$BUILD'#" "$OUT/pkg/top_web.js"
for f in "$OUT/index.html" "$OUT/app.js" "$OUT/browser.js" "$OUT/worker.js" "$OUT/pkg/top_web.js"; do
  grep -q "v=$BUILD" "$f" || { echo "version stamp missing in $f" >&2; exit 1; }
done
[ "$(grep -c "v=$BUILD" "$OUT/app.js")" -eq 4 ] || { echo "app.js: not every module stamped" >&2; exit 1; }
printf '{"build":"%s"}\n' "$BUILD" > "$OUT/version.json"
# Stop GitHub Pages from ignoring files that start with an underscore.
touch "$OUT/.nojekyll"
echo "Browser version built in $OUT ($(du -sh "$OUT" | cut -f1))"
