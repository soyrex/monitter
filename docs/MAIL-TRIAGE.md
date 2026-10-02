# Native Gmail inbox triage

Monitter's inbox reads an assigned native Gmail connection directly. It does
not ask an agent to search or read mail. Opening a message marks it read in Gmail.
The agent-facing Mail MCP remains read-only.

## Use

1. Connect Gmail in **Settings** and assign that account to the agent that owns
   the inbox chat. Configure `TYPESAFE_API_KEY` in **Settings → Environment &
   Secrets** for Jev classification (`JEV_API_KEY` remains a legacy alias).
2. Open the chat's Gmail inbox and select **Refresh**. Monitter searches the latest
   100 unread inbox messages for each assigned Gmail account.
3. Jev classifies the bounded sender, recipients, subject, timestamp, and
   snippet in background System One batches of 10. Full bodies and Gmail message
   IDs are omitted from Jev state. The UI waits for the result before showing
   any card.
4. Only messages Jev judges work-related and requiring a concrete action from
   the mailbox owner can appear. A separate judgment excludes marketing,
   newsletters/digests, and automated mail. The local gate also requires owner
   `me`, a concrete suggested action, and a non-newsletter, non-transactional,
   non-personal intent. Importance or urgency alone never admits a message.
5. Click a visible card to read that exact message through the same native
   Gmail account and mark it read. The plain-text and optional HTML body stay in a bounded process-local cache
   for up to 30 minutes and are never persisted in Snapshot or SQLite. The rich
   reader isolates email HTML; a thread digest uses source excerpts on demand.

The inbox also marks a message read automatically after a successful Jev batch
only when Jev rates its importance low, rates owner action at most 10%, rates
bulk/automation at least 95%, and suggests archive or no action. Failed or
uncertain batches leave Gmail unchanged. Existing Gmail connections need a new
Google consent through **Connect Gmail** for the `gmail.modify` scope; reconnecting
the same account keeps its Monitter assignments.

The three display judgments are Noul yes-probabilities, retained as 0–100
values in `inboxFit`: `workRelated`, `requiresMyAction`, and
`excludedBulkOrAutomated`. The display gate currently uses at least 70 for
work/action and at most 30 for the exclusion judgment. These are conservative
product thresholds to evaluate against real outcomes, not universal model
confidence rules. The five existing Choice outputs—importance, intent, reply,
owner, and suggested action—remain available for card context. Their Choice
confidence is distinct from Noul probability.

The UI does not show provisional, fallback, or legacy cards as actionable. If
Jev is unavailable or returns invalid typed answers, it displays the error
state and leaves the inbox list empty. A later refresh can retry. For
credential-free development, `MONITTER_MAIL_TRIAGE_CLASSIFIER=mock` exercises
the pipeline but likewise cannot make mock cards visible as Jev decisions.

The native connector stores body-free cards per task/account by provider
message ID. A later snapshot moves absent messages to local history; this
never changes Gmail. The owner desktop/LAN snapshot may include these bounded
envelopes, while the shared visitor projection omits all `mailBatches`.
Email content is untrusted data and cannot authorize actions. There is no
compose/send/reply/forward, archive, label, or delete operation in this inbox.

## Verification

```sh
npm run check
npm run test:mail-triage:ui
cargo test --manifest-path src-tauri/Cargo.toml mail_triage
```

The Rust fixtures check typed Jev mapping, one request per batch, bounded
body-free inputs, snapshot lifecycle, and fallback behavior. The UI fixture
checks visible-card gating and direct native detail retrieval. The ignored
`live_jev_mail_smoke_uses_synthetic_envelope` test requires a configured API
credential and is excluded from ordinary tests.
