#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
assets_dir="$script_dir/app/src/main/assets"
jdk_home="$script_dir/.toolchain/jdk-17.0.20.1+1/Contents/Home"
sdk_root="$script_dir/.toolchain/android-sdk"
gradle_home="$script_dir/.toolchain/gradle-8.7"
gradle_zip="$script_dir/.toolchain/gradle-8.7-bin.zip"

if [ ! -x "$jdk_home/bin/java" ]; then
  echo "Missing bundled JDK. Run the Android bootstrap documented in README.md." >&2
  exit 1
fi
if [ ! -x "$sdk_root/platform-tools/adb" ]; then
  echo "Missing Android SDK platform-tools. Run the Android bootstrap documented in README.md." >&2
  exit 1
fi
if [ ! -x "$gradle_home/bin/gradle" ]; then
  curl --fail --location --retry 3 --output "$gradle_zip" \
    'https://services.gradle.org/distributions/gradle-8.7-bin.zip'
  unzip -q "$gradle_zip" -d "$script_dir/.toolchain"
fi

# Vite output is owned by the desktop build. This script only packages its existing output.
if [ ! -f "$repo_dir/build/index.html" ]; then
  echo "Expected existing static bundle at $repo_dir/build/index.html." >&2
  exit 1
fi
rm -rf "$assets_dir"
mkdir -p "$assets_dir"
cp -R "$repo_dir/build/." "$assets_dir/"

export JAVA_HOME="$jdk_home"
export ANDROID_SDK_ROOT="$sdk_root"
export GRADLE_USER_HOME="$script_dir/.toolchain/gradle-user-home"
"$gradle_home/bin/gradle" --no-daemon -p "$script_dir" :app:assembleDebug
exec "$script_dir/verify-apk.sh" "$script_dir/app/build/outputs/apk/debug/app-debug.apk"
