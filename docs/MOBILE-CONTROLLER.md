# Paired mobile controller

Status: first-pass UI and encrypted session transport implemented; iOS runtime
verification remains outstanding. The hosted encrypted relay is live. Remote access is
opt-in through the desktop Remote control panel; nothing connects by default.

## Current development build

- `npm run relay`: starts the opaque development relay on 127.0.0.1:8789.
- Open Remote control in desktop Monitter, create a QR invitation, scan it on mobile, or enter the nine-digit one-use device key, then compare
  the six-digit verification numbers and approve on desktop. V2 invitations contain only a room and ephemeral desktop public key. Pairing is memory-only and disconnect requires a new invitation.
- `npm run build:ios:web`: packages the mobile web assets into the iOS project.
  Native sources and build instructions are in `mobile-ios/README.md`.
- `npm run test:controller` and `npm run test:remote`: protocol and real local
  encrypted relay tests. `npm run test:mobile` starts isolated test servers and exercises WebKit with a
  clearly test-only desktop bridge.
- Desktop and mobile have independent navigation; mobile lists existing direct
  chats, reads conversation output, sends messages, and stops running work.
- No persistent device credentials, automatic reconnect, push,
  file upload, terminal interaction UI, or mobile harness approvals are shipped.
  Loopback relay URLs work in a simulator on this Mac; a physical phone requires
  a reachable WSS relay. Native WKWebView loads bundled assets via a loopback-only
  server; that local asset server exposes no desktop controller methods.
- Disconnect blocks future controller requests. Already accepted desktop work
  continues. A failed/uncertain send is never automatically resubmitted.


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

Mobile snapshots retain conversations but limit diagnostic events to the most
recent 100, with 4,000 characters of detail each. Desktop history is unchanged.
Larger conversation histories still need pagination before broader release.

## Android pairing update

Android 0.2.0 adds the native Google Code Scanner through a main-frame,
exact-origin Web Message bridge. Scanning cancellation is quiet; scanner errors
leave the nine-digit fallback available. The fallback is registered for five
minutes, claimed once, and revoked on desktop cancellation or pairing.

The relay stores only a v2 public invitation. Both endpoints derive transport
keys using ephemeral P-256 ECDH and display a six-digit verification number;
the user must compare both displays before approving. Legacy v1 QR invitations
remain readable for migration, but cannot be registered as numeric device keys.

`npm run test:android-bundle` extracts the APK itself and checks its packaged
mobile route in Chromium. Native scanner/device testing is a separate check.
