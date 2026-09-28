# Paired mobile controller

Status: first-pass UI and encrypted session transport implemented; iOS runtime
verification remains outstanding. The hosted encrypted relay is live. Remote access is
opt-in through the desktop Remote control panel; nothing connects by default.

## Current development build

Pairing controls now live directly in **Settings → Remote control**, including the
QR/device key, number comparison, saved phones and inactivity policy. Closing
Settings or changing category hides only these controls; the root-owned listener
stays alive. This does not keep the listener alive after quitting the desktop app.
The relay must report ready before a new QR/code is offered. A socket/join attempt
is bounded to 15 seconds and remembered sessions retry with backoff; HTTP pairing
requests show readable timeout/network failures and never automatically retry a
one-use creation or claim. Scanning a QR still requires a working relay connection.

### Network-independent recovery

The paired connection uses public outbound WSS from both devices. It needs no
desktop IP address, VPN, or shared LAN. Device identities and their approvals stay
the same when either device moves networks. The iOS app starts in paired mode for
new installations; an explicitly saved manual LAN preference remains available.

Updated relays and clients negotiate a small transport heartbeat. Clients detect
an unresponsive path within 10 seconds of a probe (probes repeat every 15 seconds),
and the relay frees a stale negotiated socket after 45 seconds without heartbeats.
New connections cannot replace a live peer merely by knowing its room ID. The
Cloudflare heartbeat uses hibernation auto-response, so idle pings do not wake the
Durable Object. Lease alarms perform bounded cleanup. Existing clients remain
compatible; existing relays must be updated to enable the new liveness behaviour.

Browser lifecycle and native iOS foreground/network hints coalesce into a fresh
connection. Outstanding calls fail promptly and uncertain mutations are never
automatically sent again. A disconnected or sleeping desktop remains unavailable
until it returns; the relay does not run the desktop's agents.

`node scripts/network-recovery-run.mjs` uses real encrypted client sessions through
a local fault proxy. It drops both traffic and close notifications independently
for the phone and Mac, checks saved-identity recovery and no mutation replay, then
restarts the relay. `npm test --prefix relay-cloudflare` checks the actual local
Worker's negotiation, live role reservation, lease expiry, and compatibility.
These tests do not substitute for physical iPhone Wi-Fi/cellular and lock/unlock
verification with matching installed builds.

Release the relay before the new desktop and mobile bundles. Older peers continue
using the previous join protocol; new clients enable heartbeat frames only after
the relay advertises support. Building this source does not deploy the relay or
install either application.

After the first connection, automatic recovery keeps the workspace and drafts in
place with a small spinner. Reading, scrolling, and local navigation remain usable.
Connection feedback appears when an action needs the desktop; an offline Send keeps
the draft, and reconnection never resubmits it. The WebKit owner-relay test covers
this outage and recovery behaviour as well as foreground resume.

- `npm run relay`: starts the opaque development relay on 127.0.0.1:8789.
- Open Remote control in desktop Monitter, create a QR invitation, scan it on mobile, or enter the nine-digit one-use device key, then compare
  the six-digit verification numbers and approve on desktop. V2 invitations contain a room and desktop public key. Version 0.4.0 and its matching desktop build remember approved phones across app closure and reconnect automatically.
- `npm run build:ios:web`: packages the mobile web assets into the iOS project.
  Native sources and build instructions are in `mobile-ios/README.md`.
- `npm run test:controller` and `npm run test:remote`: protocol and real local
  encrypted relay tests. `npm run test:mobile` starts isolated test servers and exercises WebKit with a
  clearly test-only desktop bridge.
- Desktop and mobile have independent navigation; mobile lists existing direct
  chats, reads conversation output, sends messages, and stops running work.
- Push, file upload, terminal input, and mobile harness approvals are not shipped.
  Loopback relay URLs work in a simulator on this Mac; a physical phone requires
  a reachable WSS relay. Native WKWebView loads bundled assets via a loopback-only
  server; that local asset server exposes no desktop controller methods.
- Disconnect blocks future controller requests. Already accepted desktop work
  continues. A failed/uncertain send is never automatically resubmitted.

## iOS owner workspace over relay (in progress)

The iOS paired bootstrap requests a separate **full owner workspace** grant after
ordinary controller pairing. The desktop displays a distinct approval and stores
owner trust separately from remembered controller devices; a previously paired
test phone does not gain owner commands until that approval. Revoking phone access
also removes its owner grant, while revoking the owner grant alone leaves limited
controller pairing intact. Visitor/operator sessions cannot request owner scope.
The desktop must advertise `owner-workspace-v1` during its encrypted pairing
approval; otherwise the phone reports that the desktop needs an update and does
not send an unsupported request or loop reconnecting.

After owner approval, the same bundled root `AppSurface` used by the responsive
LAN site loads from the iOS shell's private loopback server. Its commands travel
encrypted through the existing Monitter relay to the desktop and are dispatched
only through the desktop's explicit owner-LAN allow-list. The relay is not an HTTP
proxy and does not receive plaintext workspace data. Owner command payloads are
bounded to 32 MiB in ordered encrypted 32 KiB chunks; uncertain mutations are not
automatically retried. LAN mode continues to load
live desktop-served assets when reachable; the bundled relay mode needs an iOS
rebuild for web UI updates. Theme mode, density, fonts and other shared Settings
come from the desktop Snapshot. The app palette, surface tint and terminal palette
are mirrored over the approved owner session because those are browser-local
preferences; mobile interface scale remains device-local. Voice remains a separate
iOS-specific parity item. This path must be signed, installed and functionally
verified on the physical iPhone before being described as complete.


## Architecture decision

Desktop owns harnesses, SSH, terminals, files, and durable conversation state. Mobile
renders a separate touch-oriented shell using shared message components and sends
explicit controller requests. Both devices connect outbound to a relay over WSS.
The relay forwards end-to-end encrypted application envelopes; it is not trusted
with conversation content. A reachable relay handles ordinary NAT/firewalls, but
blocked outbound WebSockets, captive portals, and desktop sleep must remain visible
connection states, not promises of universal connectivity.

Desktop and mobile navigation are independent. Selecting a conversation on a phone
must not rearrange desktop panes or take keyboard focus. Later, the desktop runtime
can become a standalone server using the same protocol. This does not yet imply
hosted execution or a multi-tenant service.

## Review of the proposed plan

- Reuse the existing MonitterBridge for initial dispatch. Do not route every local
  invoke method or rewrite the entire desktop UI before validating the protocol.
- Start with snapshot, existing-chat send/cancel/resume, and terminal list/read.
  Files, terminal input, creation, channel controls, and destructive operations need
  explicit capability and validation decisions in later increments.
- Snapshot currently contains private host paths, instructions, and history. Only
  an explicitly approved device may receive it. Before network release, define a
  paginated mobile projection rather than forwarding unbounded full snapshots.
- Desktop CLI adapters currently cannot answer interactive harness approvals. Do
  not ship mobile approval controls until the harness supports them end to end.
- Request IDs are required. Concurrent duplicate mutations must share one result;
  conflicting reuse must fail. In-memory deduplication alone is insufficient across
  desktop restarts. Durable receipts must accompany mutations before reconnect
  retries can be enabled in production. Unknown outcomes require reconciliation.
- A socket or TLS session is not device authorization. Pairing establishes pinned
  peer identities; authorization is checked at dispatch, including after revocation.
- Push notifications require APNs/FCM integration separate from the live socket.
  Send generic attention notifications by default, without transcript payloads.

## Delivery stages and acceptance criteria

### 1. Controller protocol and in-process proof (in progress)

Versioned envelopes, method allowlist, argument limits, typed client, bounded request
receipts, and JSON round-trip tests against a bridge double. No listener, remote
credential, relay provisioning, or automatic harness run. Tests cover malformed
requests, unsupported versions/methods, duplicate sends, conflicting IDs, backend
errors, and ordinary reads. A bridge-backed adapter keeps the existing native
implementation authoritative.

### 2. Pairing and trusted sessions

Desktop Settings exposes Remote access, initially off. QR contains a short-lived,
one-use rendezvous invitation and desktop public identity, not a permanent bearer
credential. Phone generates its own key; desktop explicitly approves the candidate
identity. Bind both identities and the invitation into an authenticated handshake
using an established audited protocol/library; do not invent encryption. Store
private keys in platform secure storage. Display paired devices and revoke them;
revocation terminates sessions and denies further dispatch. Bound pairing attempts.

### 3. Local relay and encrypted integration test

Run a development relay locally, forwarding bounded opaque envelopes with connection
limits, heartbeat, backpressure, and no payload logs. Tests include an unpaired peer,
relay tampering, replay, revoked devices, dropped connections, and reconnect. Bind
sequence numbers to a cryptographic session; reset keys/nonces safely on reconnect.
Relay routing credentials must not grant desktop controller authority.

Desktop reconnect uses backoff and jitter. Snapshot/event synchronization has a
cursor, desktop-instance identifier, and resync fallback. Persist mutation receipts
with state before permitting automatic retries. Bound retained data and pagination.

### 4. Mobile vertical slice

QR pairing, connection state, agent/chat list, conversation, send and stop. Phone
backgrounding does not stop desktop work; foregrounding reconnects and reconciles
missed events. Exercise real desktop execution through the encrypted relay before
adding terminal interaction, attachments, channels, and notifications.

### 5. Release and eventual server extraction

Secure hosted relay deployment requires an approved hosting choice; no paid resources
have been provisioned. Validate signed mobile builds and pairing/revocation on real
iOS and Android devices. Add push and desktop sleep/offline UX. Extract Rust runtime
ownership from the desktop process only as a later milestone, with explicit session
persistence and restart semantics. Hosted execution additionally requires tenant
isolation, resource accounting, and operational design.

## Cloudflare hosting adapter

`relay-cloudflare/` contains the Workers + SQLite-backed Durable Object adapter.
Clients append only room and role routing metadata to the configured `/relay` URL;
the encryption secret remains inside the QR invitation and local client memory.
The Node development relay ignores these query fields and continues to validate
its join frame, keeping local testing compatible.

Deployed on the user-confirmed Workers Free plan at `wss://api.monitter.com/relay`.
Version: `068688be-f1e4-4d92-b7a0-2f38b898f813`. HTTPS health and the real encrypted
client integration passed against the live endpoint with certificate verification.
No paid upgrade was enabled. Native device signing and real-device end-to-end
verification remain completion gates for the iOS first pass.

Mobile snapshots are controller-only projections: they retain task and agent
identities plus recent chronological conversation tails (32 messages per task,
600 non-notice messages total plus one visible marker for each truncated task)
and cap diagnostic events at the most recent 100 with
4,000 characters of detail each. Strings and retained history are measured
against a conservative sub-1 MiB serialized budget; oldest retained messages,
channel messages, and events are removed deterministically until it fits. A
system message tells the phone when a task's earlier history remains available
on desktop. Desktop history is unchanged. Protocol v1 still needs transcript
pagination before broader release.

The encrypted transport opens and seals frames in sequence, but executes up to
32 authenticated RPCs concurrently. A delayed snapshot therefore cannot block a
later send, Stop, or `peer_left` close frame. Mobile clients that offer the
`send-receipt-v1` pairing capability receive a durable `{ accepted: true }` send
receipt from a current desktop host; they refresh the snapshot independently.
Older mobile builds receive the legacy Snapshot response, and a new mobile paired
with an older desktop does the same. No send is automatically retried during this
compatibility negotiation.

## Android pairing update

Android 0.2.0 adds the native Google Code Scanner through a main-frame,
exact-origin Web Message bridge. Scanning cancellation is quiet; scanner errors
leave the nine-digit fallback available. The fallback is registered for five
minutes, claimed once, and revoked on desktop cancellation or pairing.

The relay stores only a v2 public invitation. Both endpoints derive transport
keys using P-256 ECDH with fresh connection nonces and display a six-digit verification number;
the user must compare both displays before approving. Legacy v1 QR invitations
remain readable for migration, but cannot be registered as numeric device keys.

`npm run test:android-bundle` extracts the APK itself and checks its packaged
mobile route in Chromium. Native scanner/device testing is a separate check.

## Remembered phones (Android 0.4.0 and matching desktop)

Remote control persists its host identity and owner-approved controller public
keys in IndexedDB. The phone persists its non-extractable private CryptoKey only
after a compatible desktop approves it. The relay never receives private keys.
New encrypted transports use fresh nonces on every reconnect; an encrypted
pair-request proves possession before a remembered controller is approved.
Visitor/operator sharing never inherits remembered owner access.

The panel defaults to indefinite access until revoked. A positive inactivity
limit (1–36,500 whole days) expires from the last authenticated access, renewing
the full window on subsequent access. For example, using a phone tomorrow moves
a 14-day deadline forward one day. Expired phones require manual approval again.
Saved phones show last access, expiry, and connection state with individual and
complete revocation controls. Stopping the listener retains the saved policy;
reopening the desktop restores an enabled listener. Explicit phone Disconnect
forgets its saved desktop; closing/backgrounding the app does not.

Mobile foreground, Android resume, and network restoration reconnect through a
fresh encrypted session without replaying outstanding mutations. Older desktop
builds remain usable but display an upgrade notice because they cannot retain
approval. Both the updated APK and matching desktop frontend are required.

Validation: `node scripts/pairing-storage-test.mjs` exercises Chromium and WebKit
cold browser restarts and non-extractable key persistence;
`node scripts/resumable-transport-run.mjs` exercises live relay reconnection,
revocation, and no mutation replay. After building,
`node scripts/persistent-pairing-ui-test.mjs` serves the production bundle on an
isolated local port and checks the policy editor, persisted phone list, reload
reconnection and revocation. Native Pixel lock/unlock and force-stop
validation still requires the installed app on a physical device.
