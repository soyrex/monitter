# Public shared chat client

Visitor-only browser bundle for https://share.monitter.com/share. It contains no
owner workspace UI, LAN API proxy, server secrets or stored conversations. Chat
data travels through the existing encrypted approved-peer relay connection.
The host desktop must stay open. Links are one-use; losing the session requires
a fresh invitation and owner approval. Tool approvals remain with the owner.

Build without publishing desktop/LAN assets:

```sh
npm run build:share
```

Validate/deploy using an authenticated Wrangler v4 CLI:

```sh
wrangler deploy --config share-web/wrangler.jsonc --dry-run
wrangler deploy --config share-web/wrangler.jsonc
```

This is static-asset-only hosting; there is no Worker handler, paid binding, or
plan upgrade. Do not overwrite an existing custom-domain service without checking
ownership. The relay at api.monitter.com is deployed separately; its pairing
CORS allowlist must include the exact origin https://share.monitter.com.
