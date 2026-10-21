# IPC payload audit — 2026-10-21

## Scope and delivery

Source audit of private GPUI socket methods, Tauri commands, owner LAN/relay
commands, web cache publication, controller/visitor sharing, collaboration tools,
subscription wakeups, and ambiguous-command recovery. This is a source inventory,
not a claim that every runtime response is bounded by a fixed number of bytes.

## Changed paths

| Path | Previous unnecessary payload | Current contract |
| --- | --- | --- |
| Native workspace/settings mutations | Workspace snapshot, then whole editable catalogs | Accepted/affected-ID receipt; Ideas revision only |
| Native schedules | All schedules on every save/action | Accepted and exact saved/deleted ID |
| Native preferences CAS save | Entire submitted preferences echoed back | Revision-only endpoint; reconstruct unchanged validated data locally |
| Native streaming UI changes | Full metadata projection on every revision | Negotiated ID/map/field patch; messages and channel bodies stay separate |
| Current web/Tauri/LAN streaming changes | Full metadata every revision | Opt-in metadata patch with local materialization |
| Current web/Tauri/LAN workspace mutations | Full snapshot for synchronous action results | Negotiate receipt endpoint before dispatch; read delta and return local Snapshot facade |
| Native lost-ack recovery | Workspace metadata/full history to confirm one action | Boolean authority check plus bounded target-task message page |
| Sharing/controller projection | Fetch full durable snapshot before truncation | Read existing revisioned owner cache; history through bounded pages |

IDs are allocated before native creation; consumers select that exact identity
rather than comparing whole catalogs. Agent editor reconciliation waits for an
authoritative normalized entity and preserves edits made after submission.

## Reads retained because they supply requested content

- Bootstrap and journal-gap recovery need a compact authoritative metadata base.
- Task/channel history pages are scoped to an ID, capped at 100 rows, with a
  cursor. Native startup/reset transcript inclusion is scoped to requested tasks.
- Attachment/file reads, terminal output, event detail, transcripts, usage,
  extension settings, environment-secret metadata and mail settings use their
  domain-specific results. Explicit content transfer can still be large.
- Subscriptions send a coalesced change wake/instance identity, not snapshots.
- Collaboration tools use named domain dispatch and existing per-tool bounds;
  they are not a generic workspace snapshot channel.

## Compatibility and limits

Old commands and older servers retain their old reply shapes. Only current
clients negotiate new receipts/metadata patches. No arbitrary transport error
permits replay of a mutation. Full diagnostic/compatibility snapshot endpoints
remain callable explicitly; they are no longer used by current sharing or
native command recovery. Direct old schedule commands retain their original
Snapshot contract, while current native schedules use typed receipts; no active
web schedule caller was found.

The existing 32-MiB private frame limit and 256-KiB replay receipt budget were
not raised. Metadata patch bytes count against the existing 8-MiB journal;
overflow or serialization failure resets instead of publishing incomplete state.

Known scope limitation: changed hidden-task message rows still carry text and
response metadata needed by existing context/model/collaboration summaries.
This audit removes repeated entire metadata catalogs, not every unrelated
message body. A separate explicit summary projection is needed before those
bodies can be omitted without stale summaries. Internal service Snapshot clones
also remain in some mutation implementations; they no longer cross the audited
current-client wire paths and are a separate allocation concern.

## Validation

Source review and whitespace checks completed. Coordinated `cargo check --locked
--bins` passed for the native desktop and backend. `npm run check` passed with
zero errors and 26 existing warnings. No tests were added or run; no packaging,
installation, restart or live preference interaction was performed for this audit.
