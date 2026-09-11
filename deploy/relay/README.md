# First-pass relay deployment

Prepared only. No server, DNS record, public listener, or paid resource has been created.

The relay is a Node service with one dependency (`ws`). It forwards opaque encrypted
frames between paired clients. It stores rooms only in memory and does not log
payloads. A restart disconnects both clients; this first pass requires pairing again.

## Prepare a release

Run `node scripts/package-relay.mjs` from the repository root. It produces an isolated
release directory under `artifacts/relay/` containing the server, pinned dependency
manifest, systemd service, and reverse-proxy example. It does not install or start a
service. Run `npm install --ignore-scripts --omit=dev` inside that release directory
on the target Linux server with a current supported Node.js installation.

## Configure the existing server

Use a dedicated unprivileged `monitter-relay` system user, place the release at
`/opt/monitter-relay`, and install the supplied systemd unit. Adjust `ExecStart` if
Node is installed elsewhere. The process listens only on loopback port 8789.

Place it behind your existing TLS reverse proxy. `Caddyfile.example` shows the
WebSocket proxy configuration; replace the example domain and merge it into your
existing configuration instead of replacing that configuration. The chosen hostname
must resolve to this server and present a valid certificate. Clients use
`wss://YOUR-HOSTNAME` in the desktop Remote control panel. Never advertise the
loopback development URL to a physical phone.

## Verify before sharing

Run the encrypted transport test against the chosen endpoint, pair two test clients,
check that desktop approval is required, send a message, then revoke the session.
Confirm service logs contain no pairing secrets or payloads. Reboot/restart testing
must show the clients disconnected instead of silently replaying commands.

Limits: 64 sockets / 32 rooms, bounded encrypted frames and buffered sends, join
expiry, and heartbeat cleanup. This is a private first-pass deployment, not a public
multi-tenant hosted product. Pairing secrets remain session-only; do not expose a
public signup flow or advertise persistent pairing. The host and domain still need
to be selected before installation.
