# Monitter Android shell

This is an Android WebView container for the paired owner workspace. Its bundled
`/mobile` route bootstraps pairing, then opens the same full owner relay workspace
as iPhone at the bundled root `/`. The previous limited controller remains
available at `/mobile?controller=1`.
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

The full workspace requires a separate owner approval on the desktop after
controller pairing. The bundled root uses that approved encrypted relay for
desktop operations; it does not grant owner access to the limited controller.
Desktop Whisper is the default voice mode; Settings → Conversation can select
on-device speech recognition on Android 12+ with a local recognizer. Only the
transcript enters the draft and agent message, with no WAV attachment.

Version 0.3.0's limited mobile menu remains available through the controller
fallback route. It uses the existing controller capabilities;
desktop-only operations still require the desktop app.

The mobile menu includes Standard, Activity and Projects views, channels, archived
chats, and read-only terminal, Preferences, Hosts and Agent directory views.
Navigation and drafts remain usable while controller requests are pending.
The limited controller keeps a bounded recent message tail and marks earlier
history as available in the owner workspace; terminal output retains a labelled
120,000-character tail.

`npm run test:android-bundle` tests the packaged limited-controller fallback,
using a local encrypted relay and a test-only desktop bridge. The paired owner
workspace and both voice choices are covered by
`MONITTER_OWNER_TEST_PLATFORM=android node scripts/owner-relay-ui-run.mjs`
after a direct Vite build. These checks cover pairing, menu retention
through slow refreshes, navigation/drafts during a slow send, history limits,
single delivery and disconnect. Native QR scanning still needs a physical device.

The matching host-side concurrency and fast send-receipt changes must be included
in the next desktop build to remove the older host's sequential-request bottleneck.
The APK remains compatible with that older host; building it does not install or
restart the desktop app.

Version 0.4.0 adds remembered phone approval, automatic foreground/restart
reconnection, and an inactivity policy in desktop Remote control. Access is
indefinite by default; a positive day count renews on each authenticated access.
The panel lists saved phones and can revoke each one. Install the matching desktop
changes as well: the APK alone cannot preserve an old desktop's memory-only trust.
The app retains its private non-extractable CryptoKey in IndexedDB at its existing
secure origin. Updating with the same signing key preserves this data; uninstalling
or clearing app data does not. Android Back backgrounds the app after navigating
out of its current view; it does not explicitly disconnect the paired phone.
