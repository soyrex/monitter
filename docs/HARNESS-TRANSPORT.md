# Harness stdin transport

Monitter owns one bounded stdin writer per interactive child transport. Provider
adapters continue to construct their native protocol frames; the shared writer
only applies newline framing, serializes writes, enforces admission limits, and
reports whether a frame was flushed.

## Delivery and failure

A successful send means the complete line was written and `flush` returned.
Queue admission alone is not reported as delivery. Approval and user-input
responses are sent once and are never automatically retried: after a partial
pipe write, Monitter cannot know whether the provider received the response.
If an approval response cannot be confirmed, Monitter records a visible error
and terminalizes the active turn.

Each outbound frame is limited to 32 MiB. This allows a JSON frame to carry a
stored attachment up to the existing 20 MiB upload limit when represented as
base64 (about 26.7 MiB) plus protocol text. Current Monitter user prompts append
attachment host paths rather than embedding image bytes; generated inline
images are capped at 512 KiB and attach to assistant output. The separate 2 MiB
ACP and Codex app-server limits apply to inbound line readers and are not used
as the outbound cap. Oversized outbound frames fail before queue admission.

The ordinary queue accepts at most 64 frames. All queued normal and control
frames share a 64 MiB byte budget; the control queue accepts at most four
frames and takes priority over queued ordinary traffic. The writer always
processes one frame at a time, so provider protocol order is preserved within
each queue and cancellation can overtake frames that have not started writing.

The writer thread may be blocked inside an operating-system pipe write. Closing
the mailbox never joins or waits for that thread. Stop enqueues a best-effort
priority interrupt, closes further admission, and signals the owned child
process; process teardown breaks a blocked pipe write. Dropping the final
writer handle also closes admission, while dropping an earlier clone leaves
the shared writer alive.

Callers waiting for delivery have a bounded timeout. If a frame has not started
writing, Monitter removes it from the queue. If the writer already took it,
the delivery result is unknown: it may still arrive after the timeout.
Monitter never retries that frame. It terminalizes only the exact run and turn
that requested the reply and retires that process owner, so the poisoned writer
cannot be reused by a later resident turn and a late failure cannot change a
different turn.

Writer-thread startup failures propagate to harness setup and terminate the
child instead of leaving an interactive process without an owner. Reply errors
whose request has no current run/turn fence are logged without changing task
state.

## Provider boundaries

ACP keeps ACP JSON-RPC and permission semantics in `acp_runtime.rs`. Its
session/cancel frame uses the priority lane and awaits a bounded flush before
the reader retires the transport. Permission responses await flush; failures
are recorded against the task and end the turn.

Codex app-server keeps JSON-RPC construction and request correlation in
`codex_app_server.rs`. `RunControl` owns the writer handle, so server replies,
later turns, and advisory interrupts serialize through one bounded transport.
An approval reply is considered delivered only after flush. No user prompt is
replayed after an uncertain send.

Other line-oriented provider adapters that use `RunControl::send_control` gain
the same bounded writer and explicit flush result while retaining their own
frame formats. Initial launch/bootstrap frames that must precede the
interactive transport remain provider-specific.

## Verification

The pure transport tests can run without Cargo dependencies:

```sh
rustc --edition=2021 --test src-tauri/src/outbound_transport.rs -o /tmp/monitter-outbound-transport-test
/tmp/monitter-outbound-transport-test
```

The tests cover one-time flushed delivery, bounded frame validation, priority
ordering, clone lifetime, and a fake subprocess whose unread stdin blocks until
process teardown. The broader Rust suite also exercises resident Codex turns,
ACP steering, approval lifecycle, and process teardown.
