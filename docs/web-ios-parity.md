# Web and iOS parity with GPUI

Source audit and delivery update: 2026-10-23. Source coverage and observed runtime
behavior are distinguished below.

The bundled iOS `/mobile` route requests separate owner-workspace approval and
opens the shared root workspace. `/mobile?controller=1` and visitor shares retain
their narrower permissions; those clients do not acquire owner access.

| Capability | Shared owner web / iOS status | Remaining work |
| --- | --- | --- |
| Chats, models, approvals and structured questions | Existing shared UI and backend commands | Verify actual phone flows |
| Goals, work plans, build queue and agent activity | Existing shared UI and backend commands | Verify actual phone flows |
| Ideas, workspace navigation, search and settings | Existing shared UI | Verify actual phone flows |
| Watch final assistant answer | Ported to `watch.ts` and `WatchPane.svelte` | Built and published; verify phone behavior |
| Markdown, image and PDF/HTML document panes | Viewer and bounded owner read commands implemented | Backend release built and installed; safe activation pending; verify remote reads |
| Interactive terminal | Existing full-owner terminal surface | Verify actual phone flows; limited controller remains read-only |
| Embedded browser panes | GPUI native child view unavailable remotely | Implement a remote-capable equivalent |
| Native OpenUI output | GPUI renderer; shared Markdown retains readable source | Provide equivalent supported rendering and draft-only form behavior |
| Native text input and platform shortcuts | Different platform implementations | Verify phone composition, dictation, focus and touch equivalents |

## Watch answer selection

Only idle/completed/succeeded tasks show a settled root assistant answer from
at or after the latest user message. Commentary, collaboration/subagent messages,
streaming answers and blank bodies are excluded. Selection happens before checking
streaming state, so an earlier answer does not appear while its replacement streams.

## Update route

The paired iPhone is available to this Mac through CoreDevice. The current actual
installation is `com.soyrex.monitter.ios`, version 0.4.1 build 9, installed over
build 8 without uninstalling or clearing app data. CoreDevice confirmed installation
and successful launch. Build 6 previously had an on-device launch-security error;
that is historical and did not recur with later builds. Successful launch and
remembered authorization do not prove that the workspace screen rendered.

Shared web assets publish through `scripts/publish-lan-web.mjs`; the script keeps
previous hashed chunks and atomically replaces the entry page. The iOS bundled
assets are copied by `mobile-ios/build.sh`. Live backend replacement is separate.

## Unified pairing PIN

Secure web origins and the updated iPhone owner workspace use a persistent nine-digit
desktop pairing PIN. The PIN reservation survives ordinary desktop restarts and
can be redeemed repeatedly until revoked; desktop approval is still required. Desktop verification and the separate owner grant remain
authoritative. Approved device identities reconnect without a LAN code; the PIN
is not a reusable bearer credential. A web login carries the PIN through
session storage, consumes it on the pairing page, and never puts it in a URL.
Plain HTTP LAN origins lack Web Crypto and retain the legacy LAN-code option.
The new iPhone bundle is version 0.4.1 build 7. Wireless installation and
CoreDevice launch both succeeded. The earlier build 6 launch-security error
did not recur. Actual pairing interaction remains unverified on the phone.

## Persistent PIN deployment (October 23)

Native GPUI Settings includes **Pair a phone**, backed by one retained Rust
runtime. Pairing uses no hidden WebView host. Closing Settings keeps the listener
alive. The fixed nine-digit PIN and management token are stored with the desktop
pairing identity. Relay registration retries recover the same reservation after a
lost response. Revoke all immediately disables local access before deleting the
reservation. Desktop approval and the separate owner grant remain required.

The native HTTPS/WSS startup failure was traced to an uninstalled rustls crypto
provider. The runtime now explicitly installs the existing ring provider before
starting and reports unexpected worker termination in Settings. The live desktop
record contains a nine-digit PIN. A read-only relay lookup returned HTTP 200,
`persistent: true`, null expiry, and a room and relay URL matching the saved
invitation. Access codes and private identity material were not logged. This
verifies registration, not actual phone pairing.

Web/iOS assets and the existing Cloudflare relay deployment succeeded (relay
version a6f4358f-3984-49d9-a238-a4cb799c3dcc). The iPhone build 8 signing build
succeeded. Actual interaction with the persistent PIN on the phone remains
unverified.

Desktop release `build-1792780878909` includes native Rust pairing, the TLS repair,
and the menu composition repair. It passed strict deep signature verification and
was installed at `/Applications/Monitter.app`; the installed executable hash
matches release metadata and the running GUI maps that executable. The previous
GUI was closed through its normal durable close protocol. Backend PID 77783 stayed
running; rollback copies and warm build caches were retained. No automated tests
were run.

The menu repair enables Fast window composition for chat-only and detached
workspace windows, fixes the native even-odd mask constant, and avoids duplicate
pane strokes. Its live popup appearance could not be checked after relaunch:
the computer-use tool required an unavailable session activation action. Ordinary
composer and Cmd-K typing were observed on the earlier native Rust pairing
release, not reverified on this release.

## Stable phone identity and input repair (October 23, 21:53 CEST)

Build 9 stores one mobile cryptographic identity independently of its saved
desktop connection. It migrates an existing saved key when available and persists
the identity before starting a new pairing handshake. Clearing a connection keeps
the phone identity. Controller and owner permissions remain separately approved,
but the desktop groups them into one remembered device by verified public key.
Display names do not establish identity or merge trust.

The phone launched twice using the same saved controller and owner grants. After
the second launch against the final desktop, both grants' last-access timestamps
refreshed without adding any identity or grant. One abandoned older controller
identity was retired while the GUI was stopped, with a private rollback retained.
The store now contains one device identity with two permissions. No access codes
or key material were logged. The phone's workspace rendering remains unobserved.

Owner workspace mounting no longer waits for the optional appearance request.
Owner approval remains required. Persistence and reconnect continuations check
their current session/generation before navigating or applying appearance state.

Native release `build-1792784847276` is installed and running at
`/Applications/Monitter.app`. The installed executable matches release SHA256
`6a9c75f92c184deb9404f6aaaad4887dd635d0c67d073e184306ce6845f75285`;
strict deep signature verification passed. The normal durable close protocol
saved and closed the prior GUI. Backend PID 77783 and its start time stayed
unchanged. Rollback applications and the 9.9 GiB warm cache were retained;
43.1 GiB remained free and no cleanup was needed.

The input repair isolates the working-logo animation in a retained child,
moves command-palette indexing/ranking to a coalesced background job, avoids
repeated sidebar and approval JSON clones, and stops unchanged activity-cache
updates from invalidating every retained chat pane. Ordinary key entry was
observed in an empty composer and Cmd-K search on the final installed release;
the probe characters were removed without sending a message.

Five-second samples show the repaired sidebar, approval and activity-effects
paths consume substantially fewer main-thread samples. This is not a controlled
typing-latency benchmark: overall GUI CPU remains high, and sustained typing
responsiveness still needs observation. No automated tests were run. Broad
web/iOS parity and actual phone workspace interaction remain open.
