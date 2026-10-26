# Monitter Cloudflare relay

The intended hosted endpoint is `wss://api.monitter.com/relay`. It is an opaque,
two-socket relay: the public room routing ID appears only in the WebSocket URL,
while the pairing secret and controller traffic remain encrypted application data.

## Liveness negotiation

New clients offer `relay-heartbeat-v1` in their join frame. When offered, the
relay includes that same capability in `joined` and answers the exact JSON frame
`{"type":"ping"}` with `{"type":"pong"}`. Cloudflare handles this reply through
Durable Object WebSocket hibernation, so routine heartbeats do not wake the
room. A negotiated socket is retained for at most 45 seconds after its opening
time or most recent automatic pong, whichever is later. The room alarm and a
subsequent connection attempt both remove an expired reservation before role
conflict checks.

Clients and relays that do not offer the capability keep the legacy behavior:
a completed desktop/mobile pair is long-lived. A survivor after a peer leaves
gets a fresh pending-pairing deadline; delayed close/error events from the old
socket cannot notify a new occupant.

## Alternate address

The same Worker is also available at
`wss://monitter-opaque-relay.soyrex.workers.dev/relay`. This address was enabled
on 2026-09-12 because the custom domain's DNS-selected edge addresses timed out
from the desktop's network. The existing custom domain remains enabled; preview
URLs remain disabled. No Worker code, bindings, encryption, or approval rules
were changed.

To use the alternate address, stop the listener in Settings → Remote control,
change Relay address, then restart it. This preserves the desktop identity and
remembered phones. Pair Android by scanning the new QR: numeric code entry in
the current Android client still uses `api.monitter.com`. Previously saved phone
invitations retain their old relay address and may need a fresh QR scan.

Run the local integration test with `npm test`. It starts `wrangler dev --local`,
so it needs neither a Cloudflare account nor any provisioned resource. The test
includes a real local 45-second negotiated-lease assertion, heartbeat pong,
stale-role replacement, live-role conflict, and legacy-pair compatibility.
After a separately approved deployment, `MONITTER_TEST_RELAY_URL=wss://api.monitter.com/relay npm test`
uses a fresh public room to exercise the same real encrypted-controller flow;
it deliberately skips the local-only limit and alarm checks.
If a local resolver still caches NXDOMAIN after the Custom Domain is live, add
`MONITTER_TEST_RESOLVE_IP=<Cloudflare edge IP>`; the test supplies that IP only
to Node's DNS lookup and retains `api.monitter.com` for SNI and TLS validation.

`wrangler.jsonc` prepares `api.monitter.com` as a Cloudflare Custom Domain. A
future deploy will create the DNS record and certificate only after the account
owner has confirmed that the Cloudflare zone is active and has no conflicting
CNAME. No deploy, DNS change, or account operation is performed by this project.

## Persistent desktop PINs

New desktop hosts opt in to a fixed random nine-digit rendezvous PIN. It is a
public locator and never grants device or owner access by itself; existing
verification and explicit desktop approval remain required. The desktop saves a
random management token before POST, so a lost response can recover the same PIN
by retrying with that token. Authenticated PUT updates the invitation and DELETE
revokes the locator. Legacy five-minute single-use codes remain supported.

Persistent rows have no automatic expiry and count toward the registry capacity.
A desktop must revoke its PIN before changing relays. The local Node relay has
in-memory endpoint parity only, so its registrations do not survive restart.
