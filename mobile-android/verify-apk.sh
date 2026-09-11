#!/bin/sh
set -eu

apk=${1:?usage: verify-apk.sh path/to/app-debug.apk}
[ -f "$apk" ] || { echo "APK not found: $apk" >&2; exit 1; }

entries=$(unzip -Z1 "$apk")
require() {
  printf '%s\n' "$entries" | grep -Fqx "$1" || {
    echo "APK is missing required bundled asset: $1" >&2
    exit 1
  }
}

require 'assets/index.html'
printf '%s\n' "$entries" | grep -Eq '^assets/_app/immutable/entry/start\..*\.js$' || {
  echo 'APK is missing the Svelte /_app entry JavaScript.' >&2
  exit 1
}
printf '%s\n' "$entries" | grep -Eq '^assets/_app/immutable/nodes/3\..*\.js$' || {
  echo 'APK is missing the Svelte /mobile route JavaScript.' >&2
  exit 1
}

asset_count=$(printf '%s\n' "$entries" | grep -c '^assets/' || true)
[ "$asset_count" -ge 70 ] || {
  echo "APK contains only $asset_count bundled assets; expected the complete web bundle." >&2
  exit 1
}
printf 'Verified %s bundled assets in %s\n' "$asset_count" "$apk"
