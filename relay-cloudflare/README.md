# Monitter Cloudflare relay

The intended hosted endpoint is `wss://api.monitter.com/relay`. It is an opaque,
two-socket relay: the public room routing ID appears only in the WebSocket URL,
while the pairing secret and controller traffic remain encrypted application data.

Run the local integration test with `npm test`. It starts `wrangler dev --local`,
so it needs neither a Cloudflare account nor any provisioned resource.
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
