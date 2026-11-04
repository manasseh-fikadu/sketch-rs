#!/bin/bash
# Re-renders the Chromium references for tests/fidelity.rs. Run it only when a page in
# tests/fidelity/pages changes: the PNGs are committed, so the test itself needs no browser.
#
# The pages use the app's bundled Inter, so text is comparable. `box-sizing: border-box` matches
# how every frame in the app measures itself. Fonts a page names that aren't bundled, like
# Georgia in pricing.html, depend on the machine, so regenerate all references on one machine.
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
pages=$repo/tests/fidelity/pages
refs=$repo/tests/fidelity/refs
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

faces=""
for spec in "Regular 400 normal" "Medium 500 normal" "SemiBold 600 normal" "Bold 700 normal" \
  "Italic 400 italic" "BoldItalic 700 italic"; do
  # shellcheck disable=SC2086
  set -- $spec
  faces+="@font-face{font-family:Inter;src:url(file://$repo/assets/fonts/Inter-$1.ttf);font-weight:$2;font-style:$3}"
done

for page in "$pages"/*.html; do
  name=$(basename "$page" .html)
  body=$(sed "s#AVATAR#file://$repo/tests/fidelity/avatar.png#" "$page")
  printf '<!doctype html><html><head><meta charset="utf-8"><style>%s *{box-sizing:border-box} html,body{margin:0;font-family:Inter}</style></head><body>%s</body></html>' \
    "$faces" "$body" >"$tmp/$name.html"
  timeout 90 "${CHROMIUM:-chromium}" --headless=new --no-sandbox --disable-gpu --hide-scrollbars \
    --force-device-scale-factor=1 --allow-file-access-from-files --virtual-time-budget=3000 \
    --window-size=1100,1200 --screenshot="$refs/$name.png" "file://$tmp/$name.html" 2>/dev/null
  echo "wrote $refs/$name.png"
done
