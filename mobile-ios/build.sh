#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
source_dir="$repo_dir/build"
destination_dir="$script_dir/MonitterIOS/Web"

cd "$repo_dir"
if [ "${MONITTER_SKIP_WEB_BUILD:-0}" != "1" ]; then
  npm run build
fi

if [ ! -f "$source_dir/index.html" ]; then
  echo "Expected $source_dir/index.html after the static build." >&2
  exit 1
fi

rm -rf "$destination_dir"
mkdir -p "$destination_dir"
cp -R "$source_dir/." "$destination_dir/"
printf '%s\n' "Bundled Monitter web assets in $destination_dir"
