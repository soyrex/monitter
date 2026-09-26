# Six harness architecture improvements

All six improvements were implemented and validated on `goal/harness-six-20260926` in `/Users/alex/code/monitter-harness-goal-20260926`, then integrated with local main at the user's request. They have not been installed or activated in the running application. Concurrent uncommitted main edits and user data are preserved.

| Improvement | Result |
| --- | --- |
| Native terminal authority | A separate default-off agent capability, saved task cwd, and one-time native approval gate host terminal execution. Read-only/unknown sandboxes reject it. Cancellation, revocation and replaced runs invalidate pending execution. |
| Reliable provider transport | Shared bounded JSON writers reserve control capacity, acknowledge flushed writes and close after uncertain in-flight timeouts. Errors retire only the exact owning turn. ACP's required cancelled-permission reply has a narrow exact-turn shutdown path. |
| ACP recovery identity | Saved session IDs must match every supplied recovery identity before readiness or prompt delivery. Omitted IDs retain the saved ID; malformed or changed IDs fail without replaying the prompt. |
| Proportional streaming history | Persistent indexed histories keep normal mutation/SQLite work proportional to changed rows. Revisioned task/channel deltas, bounded initial views and stable-ID pages replace full live transcript transfer. Reader freeze and anchors survive paging; authoritative edits/deletions appear on follow release. |
| Checked command contract | A manifest and generated TypeScript invocation maps cover native and owner-LAN commands. Surface/ACL/protocol drift is checked, client handshakes guard dispatch, and ambiguous mutation failures are never retried. Legacy compatibility is explicit. |
| Replayable Jev evaluation | Route records retain policy/question/model versions and full Choice distributions. Offline replay uses native route policy and imports bounded observed outcomes, preserving missing measurements as null and separating synthetic evidence from measured results. |

SQLite schema v2 normalizes channel messages into individually stored rows. Opening a v1 database migrates it transactionally; malformed identities/duplicates abort the migration. Older application versions reject v2 instead of interpreting normalized channel metadata as a missing transcript. Deployment and downgrade planning must account for that version boundary; this task has not opened the user's live database with the new code.

The command checker verifies command signatures, explicit native/LAN surfaces, Tauri grants, controller/visitor mappings, and member names for 11 core Rust/TypeScript shapes. Other nominal shapes are explicitly listed in `memberParity.unsupported`; this is not a complete serialized-schema generator. Runtime resource and actor checks remain authoritative.

Individual captured subagent transcripts remain capped at 200 entries. Per-update metadata work still scales with the number of tasks, channels and sessions. Structural history edits intentionally take a full differential/reset path; ordinary append and point-update paths are incremental. Legacy snapshot consumers retain their existing full-history behavior.

Validation uses temporary synthetic stores, fake local provider processes and browser fixtures. Live-provider tests stay opt-in; no billable model evaluation, account/configuration change, app install, restart, or live LAN publication was performed. Controlled PTY tests exercise terminal behavior after a known prompt; they do not establish readiness timing for every user's interactive shell/profile.

Detailed contracts and reproduction commands are in [HARNESS-STREAMING.md](HARNESS-STREAMING.md), [COMMAND-CONTRACT.md](COMMAND-CONTRACT.md), [JEV-EVALUATION.md](JEV-EVALUATION.md), and [CONTRACT.md](CONTRACT.md). The CI workflow runs command checks and rejection tests, Svelte checks, transcript paging, the synthetic evaluation corpus check, a direct Vite build, and the native library suite.

Use the shared Cargo target `/Users/alex/code/monitter/src-tauri/target` for local verification. Do **not** use `npm run build` in a worktree: it also publishes live LAN assets. Use `./node_modules/.bin/vite build` for an isolated web build.

The first 2,000,000-token goal reached its limit at 2,130,135 tokens and checkpointed at `225e58f`; the user then resumed with a fresh two-million-token budget. Earlier partial validation and known failures were superseded by the resumed integration work.

## Local performance evidence

The native service benchmark used 20 samples at each size on this Mac, with synthetic 4-KiB events and 2-KiB messages in temporary SQLite stores. Each measured mutation updated one task message, one channel message and appended one event. These are absolute local measurements, not a controlled before/after speedup or a service guarantee; concurrent build/test activity can affect them.

| Fixture | Mutation p50 / p95 | Delta + JSON encode p50 / p95 | Maximum delta size | Cached unchanged UI p95 |
| --- | --- | --- | --- | --- |
| 4,096 events; 1,000 task + 1,000 channel messages | 2.22 / 5.66 ms | 8.14 / 10.42 ms | 75,995 bytes | 8.04 microseconds |
| 53,000 events; 4,000 task + 4,000 channel messages | 6.48 / 11.22 ms | 11.67 / 18.84 ms | 76,057 bytes | 8.96 microseconds |

The larger store occupied 286,650,368 database bytes. Initial fixture seeding, full imports and structural resets are deliberately outside the ordinary small-update claim. Both benchmarks asserted one changed task message and one changed channel message per delta.

`cargo run --manifest-path src-tauri/Cargo.toml --bin harness -- eval` completed offline with schema `monitter-routing-evaluation-v2`, 12 cases, 24 synthetic outcomes and 12 captured-decision replays. Completion, test, cost and latency observations remained absent/null. This validates reproducible policy evaluation; it does not establish routing savings or live-model quality.

## Integration checks

- Svelte/TypeScript: zero errors, 24 warnings across seven files.
- Command manifest: 104 native and 67 owner-LAN commands; seven intentional drift cases rejected and 24 runtime compatibility assertions passed.
- Controller protocol and operator-sharing security fixtures pass, including complete history for clients without paging, once-only send acknowledgements, scope filtering and revocation.
- Task/channel paging browser fixture passes, including detached-reader freezing, authoritative edits/deletions on release, anchor preservation and delayed response after changing channels. The synthetic project-board view does not advertise channel paging.
- WebKit streaming stability and MiniMax-to-Codex transcript-switch browser regressions pass with synthetic provider fixtures. This is UI regression evidence, not a live provider turn.
- Direct Vite build passes (625 modules); the build is isolated and does not publish LAN assets.
- Offline Jev corpus validation and CLI replay pass; both native service benchmark sizes pass.

The final native library suite passes: **700 passed, zero failed, 12 ignored**, using `cargo test --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=2`. Ignored live-provider/manual checks remain opt-in; the ignored synthetic service benchmark was run separately at both sizes above. PTY fixtures use a controlled shell prompt; cleanup retries only the API's explicit stopping response within a deadline and still requires observed exit. Production terminal-close semantics are unchanged.

## Local-main integration

The combined tree includes main `bea1967` (git status/worktree display, mobile swipes and timeline readability). The sole merge conflict was the `TaskTranscript` import list; both paging and git-status types were retained. Integration fixed a temporary-string lifetime error in main's new linked-worktree detection and updated the transcript regression's structural assertion for the extracted channel component.

Combined-tree checks pass: Svelte zero errors/26 warnings, task/channel paging fixture, MiniMax-to-Codex browser regression, command manifest (104 native/67 LAN), and all eight native git tests including linked-worktree detection. The 700-test result above belongs to the original completed harness tree; the focused integration run recompiled the combined native library and tested the subsequently changed git module. No push, build publication, installation or restart is part of this local merge.
