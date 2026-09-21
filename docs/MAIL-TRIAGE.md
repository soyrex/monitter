# Mail triage MVP

This experiment keeps mailbox connectivity where it already belongs: inside
a Codex harness and its Gmail plugin. Monitter is a typed,
read-only presentation and classification layer. It never receives Gmail OAuth
credentials and exposes no mailbox mutation tools.

## Try it

1. Add `JEV_API_KEY` in **Settings → Environment & Secrets**.
2. Start a new Codex chat whose harness has the Gmail connector available.
   Existing resident sessions must be restarted to discover newly added MCP
   tools.
3. Ask, for example: `Show my important unread email since yesterday.`
4. The agent should call `mail_triage_help`, query Gmail, then put the complete
   current result (up to 20 messages, including an empty result) into one
   `present_mail_batch` call with `sync_mode: "snapshot"`. Monitter upserts the
   chat's live inbox and sends a non-empty batch to Jev in one HTTP request.
5. Click a card to request its full content. The click produces a visible
   read-only follow-up; the same agent reads that exact Gmail message and calls
   `present_mail_detail`. The panel displays plain text only.

For credential-free development, launch Monitter with
`MONITTER_MAIL_TRIAGE_CLASSIFIER=mock`. This changes only classification; a
real Gmail connector is still required for real mail. The deterministic
fallback also activates visibly if Jev is unavailable or returns invalid typed
answers.

## Data flow

```text
User mail question
  → Codex Gmail connector searches/reads bounded results
  → present_mail_batch (grant supplies task identity)
  → strict envelope validation; bodies rejected
  → upsert one body-free MailBatch per task/account by provider message ID
  → messages absent from a later snapshot move to collapsed local history
  → native cards update at the transcript foot immediately
  → one background Jev System One HTTP request for the entire batch,
    with five indexed Choice questions per email
  → same cards updated atomically with classifier evidence

Card click
  → five-minute card-specific detail grant
  → visible read-only follow-up to the same Codex chat
  → Gmail reads the exact provider message ID
  → present_mail_detail validates grant + ID and accepts plain text
  → bounded process-local cache (30 minutes, at most 40 messages)
  → native detail panel
```

The first call creates one transcript anchor. Later calls update that same
projection and do not append another mail module. `snapshot` is the scheduled
inbox default: its result must be the complete current bounded search window.
`incremental` is available when a connector query supplies only newly found
messages; it never infers that an omitted message disappeared. A zero-message
snapshot is a successful check and moves prior active cards to local history.
All lifecycle state is local to Monitter and never mutates Gmail.

The background path prevents observed Jev network latency from delaying first
paint. While it runs, cards show deterministic 45%-confidence provisional
labels and an explicit `Jev pending` badge; they change to `Jev` or a visible
fallback when the request finishes. If Monitter exits while enrichment is
pending, startup converts the durable provisional batch to an explicit
interrupted fallback instead of leaving a false perpetual-pending state.

The first Jev use in a Monitter process loads `JEV_API_KEY` from the macOS
Keychain. Monitter then keeps only that allowlisted value in a process-local
cache, avoiding repeated multi-second Keychain opens; saving or deleting the
vault through Settings refreshes or clears the cache. The value is never
serialized, returned over Tauri IPC, placed in argv, or logged. Optional
`MONITTER_JEV_DIAGNOSTICS=1` output contains only DNS/connect/TLS/first-byte/
total timings.

Jev classifies `importance`, `intent`, `reply_required`, `suggested_owner`, and
`suggested_action`. The minimum of those answer confidences becomes the card's
confidence. TypeSafe-reported provider, model, latency, token counts, and cost
are retained when supplied. Missing cost is not invented.

## Safety and privacy boundaries

- Email headers, snippets, and bodies are untrusted data, never agent
  instructions. The tool help and generated detail prompt repeat this rule.
- `present_mail_batch` accepts Gmail only, 0–20 messages, bounded headers and a
  maximum 1,500-byte snippet. The normalized aggregate state must fit the
  64 KiB Jev state budget so every non-empty batch remains one request. Unknown
  fields—including a body—are rejected.
- Jev receives only bounded sender/recipient fields, subject, timestamp, and
  snippet. Gmail message/thread IDs and full bodies are omitted. It is advisory
  and cannot change model, sandbox, approvals, Gmail permissions, or user intent.
- `present_mail_detail` is rejected until a user clicks the exact durable card.
  The grant expires after five minutes and binds task, card, and provider ID.
- A full body is normalized plain text, capped at 256 KiB, kept in memory only,
  and expires after 30 minutes. It is never serialized to Snapshot or SQLite.
- Monitter mail MCP tool payloads are replaced with a privacy marker before
  Codex activity is written to RunEvent detail.
- Shared visitors receive an explicit Snapshot projection that omits
  `mailBatches`. Authenticated owner LAN views may see body-free cards, but full
  detail commands are native-desktop only.
- There is no compose, reply, send, forward, label, archive, delete, attachment
  download, background polling, deployment, billing, or credential action.

Provider-native connector history is outside Monitter's event store and remains
subject to that harness/plugin's own privacy behavior. The MVP guarantees that
Monitter does not persist full bodies delivered through its mail tools. Use a
read-only Gmail connector scope where the provider supports one: Monitter does
not expose or invoke mutation tools, but it cannot revoke capabilities already
granted independently to the harness plugin.

## Observability

Each persisted `MailBatch` includes source/account attribution, the user-facing
query label, a classifier trace, typed card outputs, first/last seen times,
active/history state, a monotonic sync counter, and the latest added/refreshed/
history counts. A compact `mail` RunEvent records batch ID, sync mode, counts,
source, labels, and classifier evidence, but no message snippets or bodies.
Fallback mode and its bounded error reason remain visible on the card module.

## Verification

```sh
npm run check
npm run test:mail-triage:ui
node scripts/operator-sharing-security-test.mjs
cargo test --manifest-path src-tauri/Cargo.toml mail_triage
cargo test --manifest-path src-tauri/Cargo.toml collaboration_mcp
```

The ignored `live_jev_mail_smoke_uses_synthetic_envelope` test can be run
explicitly when `JEV_API_KEY` is configured. It performs one live-account
request with synthetic data and is therefore excluded from ordinary tests.

The Rust fixtures verify typed Jev answer mapping, schema rejection, body-free
serialization, detail bounds, and fallback behavior. The browser fixture checks
importance ordering, Jev attribution, the click request, plain-text display,
and dismissal. Existing operator-sharing tests verify that mail cards never
enter the visitor projection.
