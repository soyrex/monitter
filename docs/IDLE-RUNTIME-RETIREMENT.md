# Idle runtime retirement

Status: implemented in the native service. Source validation is described below;
installing a new app build is a separate operation.

## Outcome

After a chat has had no active turn for more than five minutes, release its
owned harness subprocess and disposable helpers. Keep the task, transcript,
native session ID, queued messages, configuration and native session files.
The next message restores the same native session and starts one new turn.
Successful retirement and restoration add no chat messages or warnings.
Startup latency can still be noticeable; actual restoration errors remain visible.

The unit is a task runtime, not an agent definition: one agent can have several
independent chats and processes.

## Lifecycle and collector

Use one service-owned collector, sweeping every 60 seconds. Use monotonic time
for the five-minute threshold; UI reads and protocol keepalives do not reset it.
Record `idle_since` only once a turn has ended and its durable updates have been
saved. Clear it when a send reserves the next turn. A continuously idle eligible
runtime is normally collected between five and six minutes later. Check the idle
timestamp before inspecting process ownership, so younger idle runtimes incur
only an in-memory eligibility check. Already-retiring owners remain eligible for
cleanup retries.

Track process lifecycle separately from task outcome:

`Dormant -> Active (includes starting/recovering) -> Idle -> Retiring -> Dormant`

`Idle -> Active` reuses a healthy process. A failed turn may leave a healthy idle
transport, or require transport teardown; task error status alone does not tell
us which. Transport failures invalidate the live owner without deleting the
saved session. Starting, active, recovering and retiring owners are never GC
candidates. Each owner has a generation identity.

Retirement requires all of the following:

- More than five minutes continuously idle, with a saved native session ID.
- A local macOS provider with cold-resume support: Codex app-server, Claude
  stream-json, or ACP advertising session resume/load. SSH processes remain
  pinned because local process inspection cannot verify remote background work.
- No accepted send awaiting dispatch, queued/sending message, pending start,
  steer/configuration request, approval or user-input request.
- No active tool work, continuation or owned background job that shutdown would
  interrupt. Where background ownership cannot be established, skip collection.
- No active internal-admin broker request for the internal admin runtime.

Process identities are captured before the first prompt. A successful completion
can refresh that infrastructure baseline only while the transport has never
reported tool work, accounting for MCP helpers that start lazily. Idle sweeps
also capture late startup helpers while holding the lifecycle reservation, so
a new turn cannot start during that capture. The baseline
freezes once any tool invocation is observed; unknown live descendants then pin
the runtime. This is intentionally conservative and may retain an idle runtime
whose helper starts late after tool use.

A silent tool, long reasoning turn, human approval wait or delegation wait is
still active. Never use time since the last output token as evidence of idleness.

## Retirement versus sending

All sends, including channel, collaboration, queue, internal-admin and explicit
Resume, must use one lifecycle reservation operation. Under the established
data-then-run lock order, recheck eligibility and atomically claim either a turn
or retirement. Do not hold these locks during process I/O or shutdown.

If sending wins, GC skips that owner. If retirement wins, durably retain the new
send and wait for teardown, then restore exactly once. Keep the native-session
writer reservation until the old process is reaped; a retiring owner is not an
absent owner. Coalesce concurrent wake requests so only one process starts.
An old reader may release or mutate only its own generation, never its successor.
Accepted prompts carry runtime-only receipts created with the durable running
transition. Cancellation invalidates the receipt before a waiting dispatcher can
start a process. A restart treats an accepted but unfinished turn as interrupted;
it does not replay it.

Retirement needs its own stop reason, distinct from user cancellation and a
crash. Mark the owner retiring before closing pipes. Otherwise ACP's existing
automatic transport repair could immediately relaunch a process GC just stopped.

Close the adapter's owned stdin/control queue, with a bounded deadline, then escalate
termination only for its owned process group and disposable helpers. Confirm
exit (including verified helper identities that detach or reparent), reap children,
release streams, waiters and temporary resources, revoke
owner-scoped grants, and finally release the registry entry. Never stop standalone
Monitter terminals, shared servers or independently owned jobs. Cleanup failure
keeps ownership fenced and produces a bounded diagnostic.

## Restoration

Cold restoration uses the existing native session, host, working directory,
launcher and saved task settings. Preserve CLI authentication and configuration.
Do not create a replacement session or import/replay the visible transcript when
native restoration fails. Preserve history and expose the failure for correction.

Restoring a transport is not itself a model turn. Ordinary sends restore first,
then submit the new message once. The explicit Resume action may submit the
existing continuation instruction because the user requested it. Never replay
an uncertain failed prompt automatically: it may already have executed tools.

Process-scoped approval grants expire at retirement under the current contract;
do not silently promote them to permanent grants. Durable remembered rules retain
their existing exact scope. Provider in-memory state and unsaved background jobs
are not equivalent to native conversation history and need separate handling.

## Implementation anchors

- `src-tauri/src/runtime_gc.rs`: periodic weakly owned collector, durable
  eligibility checks, owner-checked release, and bounded dispatch wait.
- `src-tauri/src/runner.rs`: `RunControl`, lifecycle clock, event-processing
  permits, ownership inspection and bounded process-group cleanup.
- `src-tauri/src/lib.rs`: accepted dispatch receipts, queue dispatch, `resume`
  and internal admin reservation. Startup starts the collector once.
- `src-tauri/src/app_server_service.rs`: durable turn completion and owner-checked
  registry release; record idle only after successful completion persistence.
- `src-tauri/src/codex_app_server.rs`: already chooses `thread/resume` when a
  native session ID exists. Do not use the user-facing Resume command to wake it.
- `src-tauri/src/acp_runtime.rs`: negotiate recovery support and distinguish
  planned retirement from unexpected pipe closure.
- `docs/CONTRACT.md`: includes internal admin retirement with an active broker
  request guard. Prompts and replies remain runtime-only.

## OpenCode failure recovery

Monitter has two
paths: legacy `opencode run --session` and OpenCode through ACP. The legacy runner
already exports session metadata to restore the original working directory.
ACP must use the recovery method advertised by the actual installed launcher.

Previously, `resume()` rejected `run_is_active()`, which only tested whether the
task existed in the run registry. An idle resident owner, including one restored
after transport loss, therefore blocked explicit Resume. This was a confirmed
code defect; it does not prove the cause of every historical failed chat.

Resume now uses lifecycle-aware reservation: reuse a healthy idle owner;
wait for an owned teardown/recovery; cold-resume when absent; reject genuinely
active turns. Do not simply remove the guard, because that could permit two
writers. Failed-chat native IDs remain saved; distinguish native
recovery errors from Monitter's own state/ownership rejection.

## Validation before rollout

1. Fake-clock tests at the five-minute boundary; active quiet turns and pending
   approvals remain alive indefinitely. Polling and renaming do not extend idle.
2. Race sends against retirement, teardown and startup: one writer, one submitted
   message, preserved queue order, and no stale-reader changes to the new owner.
3. Complete a real native session, retire its process, send again, and verify
   context continuity and the same native session ID for every enabled adapter.
4. Fail an OpenCode turn and exercise both explicit Resume and a new message,
   with live-idle, recovering and absent transports; never replay the failed turn.
5. Verify intentional retirement does not invoke ACP crash repair, retries are
   bounded, and failed restoration cannot silently create a fresh session.
6. Confirm owned child/helper PIDs exit, independent terminals/jobs survive,
   internal admin wakes correctly, and existing permission scopes are preserved.
7. Measure process-tree resident memory before retirement, after exit, and after
   wake using `get_process_metrics`; also record wake latency and failure count.

## Native validation measurements

The integrated Rust regression suite passed 379 tests, with zero failures and
six opt-in tests skipped. The native smoke test was then explicitly exercised
for both providers below. Formatting passes for all changed Rust files.

Isolated local smoke checks on 2026-09-16 exercised real Codex and OpenCode ACP
sessions using existing CLI configuration. Both retained the native session ID,
recalled a random token after cold restoration, left the saved chat snapshot
unchanged during retirement, and verified the captured process identities exited.

| Adapter | Summed process RSS released | Wake through completed reply |
| --- | ---: | ---: |
| Codex app-server | 1,649,541,120 bytes (about 1.65 GB) | 13.6 seconds |
| OpenCode ACP | 858,341,376 bytes (about 858 MB) | 7.5 seconds |

These are individual samples, not guaranteed savings or startup-only latency:
reply time includes model generation, and summed RSS can double-count shared
pages. Claude restoration, failure recovery, cancellation and lifecycle races
are covered by deterministic fixtures; no live Claude measurement was taken.
OpenCode initialization now allows 60 seconds because a native startup exceeded
the previous 20-second handshake limit during validation.

## Expected memory effect

Exiting resident harnesses and their disposable helpers should reduce their RAM
use. Savings depend on actual idle processes and must be measured; summed RSS
is an approximation because processes can share pages. This does not reclaim
Monitter's own transcript/WebView allocations or external shared services.
One-shot providers that already exit after each turn offer little extra saving.
The cost is cold startup, session-loading and helper startup on the next message.
