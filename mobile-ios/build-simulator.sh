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

# Keep the direct-Swift build path compatible with the same AppIcon catalog
# used by Xcode. Xcode compiles the catalog into Assets.car; this lightweight
# path bundles the named PNG variants declared in Info.plist instead.
icon_dir="$script_dir/MonitterIOS/Assets.xcassets/AppIcon.appiconset"
for icon in \
  "icon-20.png:AppIcon20x20.png" \
  "icon-29.png:AppIcon29x29.png" \
  "icon-40.png:AppIcon20x20@2x.png" \
  "icon-40.png:AppIcon40x40.png" \
  "icon-58.png:AppIcon29x29@2x.png" \
  "icon-60.png:AppIcon20x20@3x.png" \
  "icon-76.png:AppIcon76x76.png" \
  "icon-80.png:AppIcon40x40@2x.png" \
  "icon-87.png:AppIcon29x29@3x.png" \
  "icon-120.png:AppIcon40x40@3x.png" \
  "icon-120.png:AppIcon60x60@2x.png" \
  "icon-152.png:AppIcon76x76@2x.png" \
  "icon-167.png:AppIcon83.5x83.5@2x.png" \
  "icon-180.png:AppIcon60x60@3x.png"
do
  source_name=${icon%%:*}
  destination_name=${icon#*:}
  cp "$icon_dir/$source_name" "$app_dir/$destination_name"
done

xcrun swiftc \
  -sdk "$sdk" \
  -target "$target" \
  -parse-as-library \
  -framework SwiftUI \
  -framework WebKit \
  -framework AVFoundation \
  -framework Speech \
  -framework Network \
  -framework UIKit \
  -o "$app_dir/MonitterIOS" \
  "$script_dir"/MonitterIOS/*.swift

if [ "$build_platform" = iphonesimulator ] && command -v codesign >/dev/null 2>&1; then
  codesign --force --sign - "$app_dir"
fi

plutil -lint "$app_dir/Info.plist" >/dev/null
printf '%s\n' "Built $build_platform app: $app_dir"
