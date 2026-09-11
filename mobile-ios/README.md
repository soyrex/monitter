# Monitter iOS shell

This is a small native SwiftUI container for Monitter's `/mobile` Svelte route.
It bundles the static Svelte output and presents it in `WKWebView`; the web UI
owns all product UI and navigation. A loopback-only native HTTP server maps
`/mobile` to Svelte's SPA fallback. This lets the build keep its absolute asset
URLs and gives WebKit a trustworthy `127.0.0.1` context for Web Crypto.

## Build the web bundle

From the repository root:

```sh
./mobile-ios/build.sh
```

The script runs the normal static build, requires `build/index.html`, and
copies the complete output to `mobile-ios/MonitterIOS/Web`. This keeps asset
paths and SPA fallbacks identical to the desktop build.

## Build a simulator artifact without Xcode build

```sh
./mobile-ios/build-simulator.sh
```

This runs the asset build and invokes `swiftc` directly against the installed
iOS Simulator SDK. It writes an ad-hoc signed `MonitterIOS.app` to
`mobile-ios/.build/`, which is ignored by Git. It is useful when the Xcode
project service is unavailable; it does not boot or launch a simulator.

Set `MONITTER_SKIP_WEB_BUILD=1` only when `MonitterIOS/Web/index.html` already
contains the completed shared static build and another process owns Vite.

## Compile for physical iPhone without signing

```sh
MONITTER_IOS_PLATFORM=iphoneos MONITTER_SKIP_WEB_BUILD=1 ./mobile-ios/build-simulator.sh
```

This checks the physical-device SDK and creates `.build/MonitterIOS-unsigned.app`.
It is not installable until signed with a Personal Team provisioning profile.
Use the Xcode project below for signing and installation.

## Web-to-native QR bridge

The mobile route requests scanning with:

```ts
window.webkit?.messageHandlers?.scanQR?.postMessage(null)
```

After a scan, native code dispatches:

```ts
window.dispatchEvent(new CustomEvent('monitter:qr', { detail: invitation }))
```

The route should listen for `monitter:qr` and validate/paste the invitation
using its ordinary invitation flow. The shell deliberately does not interpret
or persist QR payloads.

## Install on iPhone with a free Personal Team

Free Personal Team signing was selected for this first pass; no paid membership or
push-notification entitlement is required. The project uses automatic signing.

1. Open `MonitterIOS.xcodeproj` in Xcode and let Xcode finish installing its required
   components. This Mac currently lacks CoreSimulator.framework.
2. In Xcode Settings > Accounts, sign in with your Apple ID.
3. In the MonitterIOS target's Signing & Capabilities, select your Personal Team.
   If the bundle identifier is unavailable, choose a unique identifier for your team.
4. Connect and trust your iPhone, enable Developer Mode when prompted, and select
   it as the run destination. Build the web assets with `./mobile-ios/build.sh`,
   then Run in Xcode.
5. For pairing on a physical phone, use a reachable WSS relay in the desktop panel.
   The default is `wss://api.monitter.com/relay`; both devices connect outbound.

The signing identity and provisioning profile are created by Xcode after sign-in;
no credentials should be entered into Monitter or stored in this repository.
