#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
build_platform=${MONITTER_IOS_PLATFORM:-iphonesimulator}
case "$build_platform" in
  iphonesimulator) target=arm64-apple-ios17.0-simulator; output_name=MonitterIOS.app ;;
  iphoneos) target=arm64-apple-ios17.0; output_name=MonitterIOS-unsigned.app ;;
  *) printf '%s\n' 'MONITTER_IOS_PLATFORM must be iphonesimulator or iphoneos' >&2; exit 1 ;;
esac
app_dir="$script_dir/.build/$output_name"
sdk=$(xcrun --sdk "$build_platform" --show-sdk-path)

"$script_dir/build.sh"

rm -rf "$app_dir"
mkdir -p "$app_dir"
cp "$script_dir/Info.plist" "$app_dir/Info.plist"
cp -R "$script_dir/MonitterIOS/Web" "$app_dir/Web"

xcrun swiftc \
  -sdk "$sdk" \
  -target "$target" \
  -parse-as-library \
  -framework SwiftUI \
  -framework WebKit \
  -framework AVFoundation \
  -framework Network \
  -framework UIKit \
  -o "$app_dir/MonitterIOS" \
  "$script_dir"/MonitterIOS/*.swift

if [ "$build_platform" = iphonesimulator ] && command -v codesign >/dev/null 2>&1; then
  codesign --force --sign - "$app_dir"
fi

plutil -lint "$app_dir/Info.plist" >/dev/null
printf '%s\n' "Built $build_platform app: $app_dir"
