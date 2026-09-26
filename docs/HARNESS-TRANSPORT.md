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

Each frame is limited to 1 MiB. The ordinary queue accepts at most 64 frames and
2 MiB of queued frame bytes. A separate control queue accepts at most four
frames (4 MiB maximum) and takes priority over queued ordinary traffic. The
writer always processes one frame at a time, so provider protocol order is
preserved within each queue and cancellation can overtake frames that have not
started writing.

The writer thread may be blocked inside an operating-system pipe write. Closing
the mailbox never joins or waits for that thread. Stop enqueues a best-effort
priority interrupt, closes further admission, and signals the owned child
process; process teardown breaks a blocked pipe write. Callers waiting for
delivery have a bounded timeout. A timeout is an unknown delivery result, so
callers fail visibly instead of replaying the frame.

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
ordering, and a Stop path that closes admission promptly while a fake pipe
writer is blocked. The broader Rust suite also exercises resident Codex turns,
ACP steering, approval lifecycle, and process teardown.
