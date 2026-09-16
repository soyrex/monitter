# Idle runtime retirement

Status: proposed design; runtime behavior is not changed by this document.

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

Use one service-owned collector, sweeping every 30 seconds. Use monotonic time
for the five-minute threshold; UI reads and protocol keepalives do not reset it.
Record `idle_since` only once a turn has ended and its durable updates have been
saved. Clear it when a send reserves the next turn. A continuously idle eligible
runtime is normally collected between five and five-and-a-half minutes later.

Track process lifecycle separately from task outcome:

`Dormant -> Starting -> Active -> Idle -> Retiring -> Dormant`

`Idle -> Active` reuses a healthy process. A failed turn may leave a healthy idle
transport, or require transport teardown; task error status alone does not tell
us which. Transport failures invalidate the live owner without deleting the
saved session. Starting, active, recovering and retiring owners are never GC
candidates. Each owner has a generation identity.

Retirement requires all of the following:

- More than five minutes continuously idle, with a saved native session ID.
- A provider with verified cold-resume support for this transport and host.
- No accepted send awaiting dispatch, queued/sending message, pending start,
  steer/configuration request, approval or user-input request.
- No active tool work, continuation or owned background job that shutdown would
  interrupt. Where background ownership cannot be established, skip collection.
- No active internal-admin broker request for the internal admin runtime.

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

Retirement needs its own stop reason, distinct from user cancellation and a
crash. Mark the owner retiring before closing pipes. Otherwise ACP's existing
automatic transport repair could immediately relaunch a process GC just stopped.

Ask the adapter to close gracefully, with a bounded deadline, then escalate
termination only for its owned process group and disposable helpers. Confirm
exit, reap children, release streams, waiters and temporary resources, revoke
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

## Current implementation anchors

- `src-tauri/src/runner.rs`: `RunControl`, process ownership and bounded cleanup;
  add lifecycle, idle clock, stop reason and turn/retirement reservation here or
  in a dedicated runtime lifecycle module.
- `src-tauri/src/lib.rs`: `send_to_resident`, `launch_accepted`, queue dispatch,
  `resume`, `run_is_active`, `reserve_run` and internal admin; unify reservation
  and start/stop the collector with the service.
- `src-tauri/src/app_server_service.rs`: durable turn completion and owner-checked
  registry release; record idle only after successful completion persistence.
- `src-tauri/src/codex_app_server.rs`: already chooses `thread/resume` when a
  native session ID exists. Do not use the user-facing Resume command to wake it.
- `src-tauri/src/acp_runtime.rs`: negotiate recovery support and distinguish
  planned retirement from unexpected pipe closure.
- `docs/CONTRACT.md`: currently promises the internal admin has no idle timeout;
  implementation must explicitly revise this rule if the admin is collected too.
  Recommended policy is to include it with the broker reservation guard above.

## OpenCode failure recovery

Investigate and validate this before enabling GC for OpenCode. Monitter has two
paths: legacy `opencode run --session` and OpenCode through ACP. The legacy runner
already exports session metadata to restore the original working directory.
ACP must use the recovery method advertised by the actual installed launcher.

One confirmed code-level mismatch is that `resume()` rejects `run_is_active()`,
while that function tests only whether the task exists in the run registry.
An idle resident owner, including one restored after transport loss, therefore
blocks explicit Resume. This is a candidate for the reported symptom, not a
verified diagnosis of Alex's particular failed chat.

Replace this check with lifecycle-aware reservation: reuse a healthy idle owner;
wait for an owned teardown/recovery; cold-resume when absent; reject genuinely
active turns. Do not simply remove the guard, because that could permit two
writers. Verify failed-chat native IDs remain saved and distinguish native
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

## Expected memory effect

Exiting resident harnesses and their disposable helpers should reduce their RAM
use. Savings depend on actual idle processes and must be measured; summed RSS
is an approximation because processes can share pages. This does not reclaim
Monitter's own transcript/WebView allocations or external shared services.
One-shot providers that already exit after each turn offer little extra saving.
The cost is cold startup, session-loading and helper startup on the next message.
