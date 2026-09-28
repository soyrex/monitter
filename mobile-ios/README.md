# Monitter iOS shell

This is a small native SwiftUI container with two connections. On a fresh
launch it opens the paired relay workspace first, so a previously paired phone
reconnects over Wi-Fi or cellular without selecting a network or entering an
IP address. If the app returns from the background or the iPhone changes
network path, the shell asks the already-loaded page to resume once; it keeps
the same WebKit instance, origin, pairing keys, and page state instead of
reloading it.

- **Owner LAN workspace (advanced)** opens the same live responsive site as a phone browser.
  Enter the address shown in desktop Settings → LAN access, such as
  `http://192.168.1.20:18436/`. The Mac must be running and reachable on the
  same network. The site and its owner permissions come from the desktop LAN
  server, so its web updates appear here without an iOS rebuild.
- **Owner workspace over relay** uses the existing end-to-end encrypted Monitter
  relay away from the LAN. The bundled `/mobile` route handles pairing, then
  opens the same responsive root workspace used by LAN after a separate,
  explicit full-workspace approval on the desktop. A loopback-only native HTTP
  server gives both routes a stable origin for WebKit pairing state. Existing
  limited controller grants are not automatically upgraded. An older desktop
  without the `owner-workspace-v1` capability is reported as needing an update,
  without repeatedly disconnecting the paired connection.

The connection chooser saves only the LAN base address, never an access code.
Choosing local LAN is explicit and remains the saved preference until changed;
the app never upgrades or changes that trust decision. If that local address is
unavailable away from the LAN, choose the paired workspace to use the saved
relay pairing instead.

## Build the web bundle

From the repository root:

```sh
./mobile-ios/build.sh
```

The script runs the static Vite build without publishing LAN assets, requires
`build/index.html`, and copies the complete output to
`mobile-ios/MonitterIOS/Web`. This keeps the relay workspace's asset paths and
SPA fallbacks identical to the desktop build.

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

## Focused lifecycle check

```sh
node scripts/network-recovery-run.mjs
npm run test:owner-relay-ui
```

These exercise silent connection failures and foreground recovery in the real
encrypted client and WebKit owner route. Build the web assets first for the UI
test. Use `./mobile-ios/build-simulator.sh` to compile the native lifecycle hooks
against an installed iOS SDK. Physical Wi-Fi/cellular and lock/unlock checks
remain separate from these local tests.

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
5. For the owner LAN workspace, enter one of the desktop's Settings → LAN access
   addresses in the app. The phone must be on the same LAN. For the remote
   owner workspace, use a reachable WSS relay in the desktop panel. The default
   is `wss://api.monitter.com/relay`; both devices connect outbound. Approve
   both the limited phone pairing and the separate owner workspace request on
   the desktop.

The signing identity and provisioning profile are created by Xcode after sign-in;
no credentials should be entered into Monitter or stored in this repository.
