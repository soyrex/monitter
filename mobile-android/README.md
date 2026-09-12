# Monitter Android shell

This is a thin Android WebView container for the shared static `/mobile` route.
It builds fresh web assets with Vite and packages this checkout's `build/`
directory. It does not run the desktop build or publish to the desktop LAN server.
The app serves that bundle at Android's secure
`https://appassets.androidplatform.net` origin through `WebViewAssetLoader`, so
the web UI can use Web Crypto and outbound `wss://api.monitter.com/relay`.

## Build a debug APK

```sh
./mobile-android/build-debug.sh
```

Use a dedicated Git worktree when desktop development is active. Dependencies,
web output, Android app output and Gradle cache should belong to that worktree;
the installed JDK, SDK and Gradle distribution can be reused. Preserve the existing
debug signing key when building an update for an already installed app.
Pass `--existing-web` only to package an already verified web build from this checkout.

The script expects the free JDK 17 and Android SDK 35 bootstrap under
`mobile-android/.toolchain/`, copies the current static assets, and produces:

```
mobile-android/app/build/outputs/apk/debug/app-debug.apk
```

This debug APK uses Android's existing debug signing key (normally
`~/.android/debug.keystore`). It is suitable for installation on a Pixel
or by opening the APK on the phone and allowing installation from that source.
Pair with the desktop using the native QR scanner or nine-digit device key,
then compare the verification numbers and approve on desktop.

Version 0.3.0 mirrors the desktop sidebar in the paired mobile menu while keeping
mobile navigation independent. It uses the existing controller capabilities;
desktop-only operations still require the desktop app.

The mobile menu includes Standard, Activity and Projects views, channels, archived
chats, and read-only terminal, Preferences, Hosts and Agent directory views.
Navigation and drafts remain usable while controller requests are pending.
Transcripts initially render 100 recent messages with an explicit earlier-history
control; terminal output retains a labelled 120,000-character tail.

`npm run test:android-bundle` tests the actual packaged assets, using a local
encrypted relay and a test-only desktop bridge. It covers pairing, menu retention
through slow refreshes, navigation/drafts during a slow send, history limits,
single delivery and disconnect. Native QR scanning still needs a physical device.

The matching host-side concurrency and fast send-receipt changes must be included
in the next desktop build to remove the older host's sequential-request bottleneck.
The APK remains compatible with that older host; building it does not install or
restart the desktop app.
