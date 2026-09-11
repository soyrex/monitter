# Monitter Android shell

This is a thin Android WebView container for the shared static `/mobile` route.
It packages the existing repository `build/` directory without running Vite.
The app serves that bundle at Android's secure
`https://appassets.androidplatform.net` origin through `WebViewAssetLoader`, so
the web UI can use Web Crypto and outbound `wss://api.monitter.com/relay`.

## Build a debug APK

```sh
./mobile-android/build-debug.sh
```

The script expects the free JDK 17 and Android SDK 35 bootstrap under
`mobile-android/.toolchain/`, copies the current static assets, and produces:

```
mobile-android/app/build/outputs/apk/debug/app-debug.apk
```

This debug APK is signed with the stable Android debug keystore stored under
the ignored Gradle toolchain cache. It is suitable for installation on a Pixel
with USB debugging enabled. The initial Android pass uses the mobile route's
existing pairing-code paste flow; QR scanning remains native-iOS-only for now.
