# Monitter validation

Validation of the first locally installed macOS build. This document distinguishes real CLI
execution from browser fixtures and records the limits of each check.

For the subsequent Rust HTTP collaboration MCP migration, see
[RUST-HTTP-MCP.md](RUST-HTTP-MCP.md). Historical Python relay/model-turn evidence below
does not, by itself, prove the new HTTP path.

## Verified environment (2026-09-09)

- Mac: macOS 26.1, arm64, Xcode installed, Rust 1.94.0, Node 26.8.1.
- Local Codex: `/Users/alex/.local/bin/codex`, version 0.153.2, logged in using ChatGPT.
- SSH alias `mira`: Linux x86_64, home `/home/alex`.
- Mira Codex: `/home/alex/.npm-global/bin/codex`, version 0.130.0, logged in using ChatGPT.
- Both versions document `exec --json`, `exec resume`, stdin prompts, configuration overrides,
  and read-only/workspace-write sandboxes. Resume flags must respect their command position.
- Dedicated Mira smoke folder created: `/home/alex/.local/share/monitter/smoke-workspace`.
- Production npm dependency audit: zero reported vulnerabilities at initial setup.
- Mira Python 3.12.3 is available for remote process supervision and the smoke process check.

## Browser checks completed

`npm run test:ui` uses an injected test-only bridge. The initial twenty interaction checks passed, with no
page errors: creating agents/tasks, send/stop, retaining a draft after a visible transport error,
theme/custom accent, creating a channel, selected-recipient routing, SSH host add/probe/config
defaults, independent versus delegated tasks, native session ID attachment, sanitized Markdown,
per-task draft isolation, simultaneous runs with independent Stop, modal keyboard
containment/Escape/focus restoration, a 50KB diagnostic collapse/expand with bounded scrolling,
and a clean browser preview without the native bridge. Follow-up checks cover saved interface scale,
independent tool/reasoning display toggles and expandable blocks, task and channel send shortcuts,
Shift+Enter/IME behavior, and multiple header tabs whose closure preserves the underlying tasks.

The current source passes 27 browser interaction checks with no page errors. These include Cmd-K
navigation and archive restoration, Cmd-P toggles, empty-agent chat creation, confirmed deletion,
goal/computer lifecycle fixtures, bounded panes and palettes at 512×340, and switching chats while
task/channel sends are pending without losing unrelated drafts. Light/dark screenshots and the
final centered palettes were visually inspected. This checks the UI only; it does not prove that
Codex or SSH execution works. The current frontend type check has zero errors/warnings; the earlier
installed frontend passed its production build. The report is `verification/ui-results.json`.

## Native checks completed

- `cargo test --all-targets -j4`: 36 tests passed on the current source; main/smoke test targets also
  compiled. Covers command construction, escaping, inherited
  session environment, JSONL parsing, atomic/private/corrupt state, transactional mutation failures,
  immutable task hosts, restart interruption, single session writers, local process-group stop,
  the Python supervisor stopping a sleeping child on control EOF, old-settings migration,
  settings serialization/roundtrip, interface-scale validation boundaries, provider parsing/resume
  arguments, goal response handling, computer event metadata, provider/host session keys, archive
  preservation/send guards, and bounded probes with provider-neutral errors.
- `npm run test:hermes-bridge`: four offline cases passed: installation/venv discovery,
  create/resume/approval and event sequencing, process-group cancellation, and bounded shutdown of
  an unresponsive gateway. These use fake gateways and do not prove a live Hermes model turn.
- Local real Codex first turn and remembered-marker follow-up: passed, two assistant messages,
  one preserved native session, persisted across reopening the service.
  Evidence: `verification/native-local-1788975832866/evidence.json`.
- Mira real Codex first turn and remembered-marker follow-up: passed with the final supervisor,
  after its CLI login was refreshed. Evidence: `verification/native-mira-1788976403805/evidence.json`.
- Missing executable: visible error persisted; `verification/native-error-1788975608875/evidence.json`.
- Local cancellation after a real tool event: interrupted state persisted;
  `verification/native-cancel-local-1788976403803/evidence.json`. No process retained the local
  smoke workspace as its cwd in the follow-up `lsof` check.
- Mira cancellation after a real tool event: interrupted state persisted; a `/proc` check found
  no remaining processes at all in the dedicated remote workspace.
  Evidence: `verification/native-cancel-mira-1788976504647/evidence.json`.
- Codex Desktop successfully read both completed turns of a Monitter-created native CLI session.
  Evidence: `verification/desktop-native-history.json`. This proves shared history readability,
  not live takeover of an independently running Desktop process.

The first live continuity check found identical responses being deduplicated across turns; that was
fixed and the local/remote continuity checks then passed. The first remote cancellation check
failed when a Codex process survived the reported interruption; the supervised control connection
fixed that and the stronger cancellation check now passes.

## macOS package

- `CARGO_BUILD_JOBS=4 npm run build:mac:local`: passed. Native development profile with bundled
  production frontend; the app does not depend on Vite or a local web server.
- `npm run install:mac -- --debug`: installed and launched `/Applications/Monitter.app`.
- Installed app passes `codesign --verify --deep --strict`; its process is running.
- The development server was stopped before installation/launch.
- Native startup created the private profile directory at
  `~/Library/Application Support/com.monitter.desktop/`, with durable state in
  `state.sqlite3` (mode 0600) and `state.json` retained as the explicit SQLite
  downgrade guard. Inspect the whole directory, including SQLite `-wal` and
  `-shm` sidecars, when preserving or restoring a profile.
- Latest DMG path is recorded in `artifacts/macos-debug.json`; package manifest: `artifacts/macos-debug.json`.
  `hdiutil verify` passed. Installed app: 34 MB; compressed DMG: 11 MB.
- Documents File Provider adds FinderInfo that prevents bundle signing there. The build script
  copies this project's compiled app to `~/Library/Caches/Monitter/builds/`, removes only
  FinderInfo/ResourceFork signing metadata, signs ad hoc and verifies it before producing the DMG.
  Quarantine and Gatekeeper settings are not changed. This is a local build, not a notarized release.

## Installed native interface

- The installed application's native WebView was visually inspected after the Mac was unlocked.
  It loads `tauri://localhost`, renders the supplied design, and operates without a dev server.
- Mira was added using the native Hosts form. Its Probe reported "Connection ready" and
  `codex-cli 0.130.0`. The saved "Codex on Mira" agent uses this SSH host and the dedicated folder.
- Alex's local task completed actual Codex replies through the native composer. Its run detail
  displayed the native session ID, real tool activity, usage, and CLI diagnostics. Private task
  content is deliberately not copied into this validation report.
- A dedicated native "Mira connection check" task completed with "I am the Monitter agent for
  Mira." The prompt asked it to identify itself from its saved instructions without tools. The UI
  showed running state, incoming events, session ID, the completed reply, and usage.
- Changed the accent to violet in the native Appearance controls, quit normally after all tasks
  finished, and reopened the installed app. A new process restored the same complete state-file
  SHA-256, all hosts/agents/tasks/messages/events, and the violet accent. The restored conversation
  was opened in the native UI. Original green was then restored using Appearance.
  Evidence: `verification/native-ui-evidence.json` (only the dedicated test content is included).

## Integrated window header

- Alex requested removal of the separate native title bar after the initial native validation.
  Tauri's macOS overlay style retains native window controls inside Monitter's header. The window
  title is hidden, space is reserved for the controls, and passive header areas are drag regions.
- Reference: https://v2.tauri.app/learn/window-customization/
- Task tabs now occupy the header in place of the breadcrumb. Opened tasks remain available as
  independent tabs; closing a tab preserves the task and its draft.
- Appearance adds native WebView zoom (80–200%, default 125%), separate tool/reasoning visibility, and a shared
  task/channel Enter-to-send preference. The default remains Cmd+Enter, with composition events
  ignored to avoid accidental submissions while using an IME.
- Header/scrolling/scale package was rebuilt and installed. Subsequent palette, toggle, archive and
  harness changes were packaged after Alex returned; see the latest installation entry below.

## Scope and operational limits

- The installed app includes Codex, Claude, OpenCode and Hermes adapters. Codex has live validation;
  the new adapters have not yet been validated against live accounts.
- The CLI provides structured item/turn events; token-by-token text is not promised by exec mode.
- Completed native sessions can be resumed by ID. Older transcript import and control of another
  live Desktop/terminal process are not implemented.
- Existing CLI plugin/MCP diagnostics are surfaced in the run detail. A failed optional connector
  is separate from an authenticated Codex conversation; Monitter does not repair account login.
- The installed package uses the Rust development profile and is locally signed, not notarized.
- Multiplayer channels and iOS are intentionally deferred.

Browser UI fixtures, if used, are test-only and never evidence of real agent execution. Actual
CLI smoke tests must run the same service used by Tauri commands. No mock replies ship in the app.

## Code iteration while Alex is on phone

- The scroll/scale update was already installed before the phone instruction. Its source bundle is
  `~/Library/Caches/Monitter/builds/debug-JLb7bp/Monitter.app`, built 2026-09-09T20:27:04Z.
  `verification/scroll-update-before.json` confirms identical state-file SHA-256 before/after;
  user-created agents, chat content and current preferences were preserved.
- Later code adds accessible toggle switches, Cmd-K navigation, Cmd-P controls, confirmed chat
  deletion, reversible archives and empty-agent new-chat controls. Command search prioritises exact
  labels above fuzzy matches in descriptions. Pending sends clear only their own submitted draft.
  New UI regression checks are in `scripts/ui-smoke.mjs`.
- Native adapters now dispatch actual Claude/OpenCode CLIs and the Hermes TUI gateway, use session
  resume IDs, preserve native host permissions, and share owned local/SSH process cancellation.
  No Claude/OpenCode/Hermes live model prompt was sent during this iteration.
- Hermes protocol fixtures cover its full-duplex bridge; installed wrapper discovery is checked
  without a model call. Preserve the venv Python path rather than resolving its interpreter symlink.
- An actual read-only Codex 0.153.2 app-server initialize + thread/goal/get request succeeded on
  2026-09-09. Returned fields included objective, status, tokenBudget, tokensUsed and timeUsedSeconds.
  No thread resume, goal mutation or model turn was performed. Older remote Codex versions may lack
  this API; the UI shows unavailable lookup details rather than invented goal state.
- Computer activity uses reported lifecycle IDs; tool wrappers that hide their computer operation
  cannot be detected reliably. Hermes free-form goal text is displayed as a reported update.
- App packaging and native checks were deferred during the phone interval, then resumed below.
  Live new-harness first/resume/tool/cancel checks remain. `scripts/provider-smoke.mjs` prepares
  isolated checks through the same native Service.

## Latest installation after phone interval

- Built application source `a9e01af` using `CARGO_BUILD_JOBS=4 npm run build:mac:local`; manifest
  timestamp `2026-09-09T21:06:01.144Z`. Native compilation, bundle signing and disk-image verification passed.
- Installed to `/Applications/Monitter.app` and restarted normally. The installed executable's
  SHA-256 matches the packaged binary; strict signature verification passed.
- Verified native Cmd-K opens the channel/chat/agent switcher and Cmd-P opens controls with current
  toggle states. The sidebar displays the new plus, archive and delete controls.
- Preserved both agents, all five tasks and all existing transcript/configuration fields. Alex
  adjusted interface scale after relaunch; this was the only changed pre-existing state field.
  A private pre-install state backup and previous application bundle are retained in artifacts.
- Evidence: `verification/latest-install-evidence.json`. No live new-harness prompt was sent.

## Conversation scrolling

- `node scripts/ui-scroll-smoke.mjs`: eight focused browser checks passed with no page errors.
  Chats/channels open at latest, scrolling up exposes the jump button, incoming content respects
  history reading, late content growth/pane resizing follows the bottom, and sends reveal their
  message without scrolling a different chat after a delayed acknowledgement.
- The existing 27 browser checks also passed; frontend typecheck has zero errors/warnings.
- Evidence: `verification/ui-scroll-results.json` and `verification/ui-results.json`.
- Source `1f7338b` was packaged, signed, installed and restarted after the active chat finished.
  Native checks confirmed the latest reply is visible on activation, scrolling up exposes the
  down arrow, and clicking it returns to latest. Saved state remained byte-identical across restart.
  Installation evidence: `verification/message-scroll-install-evidence.json`.


## Projects, draft chats and compact controls (2026-09-10 local time)

Source commit `1387e4e` adds independent projects with optional per-host folders, saved sidebar
views, draft-first new chats, the slash app-action menu, scale keyboard shortcuts, local avatars,
compact chat headers and the monitter dropdown.

- Frontend typecheck: zero errors and zero warnings.
- Rust: 43 tests pass. New coverage includes project migration/validation/roundtrip, multi-agent
  and per-host folder selection, preserving existing task runtime/history during assignment and
  project deletion, and avatar migration/format/size validation.
- 55 browser fixture checks pass with no page errors: 28 general interactions, seven Projects
  cases, 12 controls/draft/header/avatar cases and eight scrolling cases.
- Projects were checked against the production frontend on port 18421. The development server
  received delayed File Provider notifications for generated Svelte files, which reloaded a prior
  test page; using the production preview removed that test-environment interruption.
- Draft checks prove no task creation before Send, no duplicate task on first-send retry, retained
  text/selection across tabs, and preserved new typing/navigation during pending creation.
- Control checks cover 5% Cmd+plus/minus increments, rapid queued presses, 80–200 bounds, slider
  synchronization, slash keyboard and Send-button dispatch, literal escaping, compact headers,
  local image selection/removal, and bounded overflow at a 512×340 viewport.
- Evidence: `verification/ui-results.json`, `verification/ui-project-results.json`,
  `verification/ui-controls-results.json`, `verification/ui-scroll-results.json`.
- These tests use isolated fixtures and do not start real agents or change user data. No new live
  harness prompts were sent. Native execution remains covered by the earlier local/Mira checks.
- The design audit is `docs/DESIGN_VIEW_AUDIT.md`. Cross-harness discovery/delegation is researched
  in `docs/AGENT_DELEGATION_FEASIBILITY.md`; that protocol is proposed, not implemented.
- Packaged source `1387e4e` successfully with `CARGO_BUILD_JOBS=4 npm run build:mac:local`.
  Bundle: `~/Library/Caches/Monitter/builds/debug-5Q0Dwp/Monitter.app`; manifest timestamp
  `2026-09-09T22:09:32.051Z`. Strict ad-hoc signature and DMG checksum verification passed.
- Installed and reopened `/Applications/Monitter.app` after Alex confirmed it was closed.
  The installed binary matches the signed package; strict signature verification passed.
  Saved state remained byte-identical, preserving four agents, two hosts, seven chats and all
  messages, events, channels and preferences. Private state and previous-app backups are retained.
- The new installed app process was verified. Native visual inspection was unavailable because
  the Mac was locked; no model prompt or UI preference change was made during installation.
  Evidence: `verification/projects-package-evidence.json` and
  `verification/projects-install-evidence.json`.


## Cross-agent messaging and discovery (2026-09-10)

Installed source: `3953dfc87f7b8e9beb7915d17a995809c8fc6e30`. This replaces the earlier
feasibility-only status: saved agent capabilities, harness discovery, callable peer messaging,
durable delegation/results and visible task lineage are implemented.

- `cargo test --all-targets -j4`: **61 tests passed**. Collaboration coverage includes authenticated
  caller scope, forged/revoked grants, malformed/oversized/fragmented HTTP, bounded concurrency,
  idempotency, enabled recipients, project/host mapping, active-writer queues, reply addressing,
  cancellation, cycles/budgets, result markers, inbox acknowledgement and restart uncertainty.
- Python MCP protocol test passed; all four Hermes gateway offline checks passed.
- Frontend typecheck: zero errors/warnings. Production build passed. **60 browser checks passed**
  across general UI (28), Projects (7), controls (12), scrolling (8) and collaboration (5).
  Final collaboration checks include actual multiline typing, legacy profile-array migration,
  availability switches, Delivered versus completed, partial-result/error display and linked chats.
- Real Codex proof with two agents on **Mira**, through Monitter's Mac service and two owned SSH
  reverse forwards: **passed**. The coordinator discovered its peer, delegated exactly one task,
  received exactly one peer inbox message and the actual result, then resumed its native session
  and read the persisted exchange. Exactly two tasks and distinct native session IDs existed;
  no duplicate delivery turn or delegation was created. Both tasks completed.
  Evidence: `artifacts/collaboration-live-mira-to-mira-1789002233555994000/result.json` and its
  private isolated state. Native sessions: `01a088d7-c014-7f73-9903-116e282cd554` and
  `01a088d8-6e9a-7031-96a2-1b09611476b9`.
- Mira's installed Codex 0.130 rejects GPT-5.6 Luna as requiring a newer CLI. The successful proof
  used its configured compatible GPT-5.5; account authentication and global configuration were
  unchanged. Existing model-catalog/skill diagnostics remained visible and did not block this proof.
- Live testing exposed and fixed an SSH stdout/exit-status race and macOS inheritance of
  nonblocking accepted sockets. The latter caused immediate intermittent 408 replies; explicitly
  restoring blocking reads and a delayed-fragment regression fixed the real routed traffic.
- After completion, no `monitter-mcp.*` directories remained on Mira. No remote daemon or public
  listener was installed. Proof state contains only dedicated test agents/chats and host definitions.

### macOS installation

- Rebuilt and installed `/Applications/Monitter.app`; signed bundle and DMG verification passed.
  Package: `~/Library/Caches/Monitter/builds/debug-6Ua8tD/Monitter.app`, built
  `2026-09-10T01:05:19.101Z`. The app was restarted after verifying all existing tasks were idle.
- Installed executable SHA-256:
  `854f00a838877a515e963d7cf09bb82dc273e9a0b090f1f7bad45ad20f1b7df0`.
  It matches the signed package. The installed process is running with its broker on loopback only.
- Saved state remained **byte-identical**, preserving all four agents, two hosts, eight chats and
  their messages/events/preferences. Private state and previous-application backups were retained.
  Evidence: `verification/cross-agent-install-evidence.json`.

### Remaining validation and environment blocker

The complete **local Mac Codex-to-Codex** proof and mixed Mac/Mira proof are still pending. Normal
Codex startup blocked while reading iCloud-offloaded
`/Users/alex/Documents/opencode-harness/.agents/skills/delegate-to-opencode/SKILL.md` and
`agents/openai.yaml`. A process sample and open-file evidence confirmed the blocked reads before a
native session began. The bounded test timed out, marked its task interrupted and reaped its owned
process. This is not a passed local handoff proof; no native configuration was disabled to mask it.
Earlier local normal-config MCP registration worked before these files were evicted.

The Mac was locked, so Finder pinning and final native visual inspection were unavailable. Unlock
and keep `/Users/alex/Documents/opencode-harness` and the original desktop repository downloaded,
then run `python3 scripts/collaboration-smoke.py --peer local` and `--peer mira` from the active
checkout. See `WORKSPACE_RECOVERY.md` for the safe source location and Git ancestry recovery.

Claude/OpenCode outbound tool injection is implemented and structurally checked; live model handoff
through those harnesses remains unclaimed. Hermes receives delegated tasks through its existing
adapter, but outbound callable collaboration awaits its ephemeral tool interface. Multiplayer and
iOS remain outside this version.


## 2026-09-10: desktop workspace update

The macOS build at 08:12 UTC adds independent split panes and draggable tabs/dividers, the compact
sidebar and narrow-screen detail blade, seamless active tabs, archive-first chat management, Git
status/diffs, startup/Stop state, centered jump-to-latest, and file/image drop/paste/picker support.
Resume now starts a real continuation using the existing native session ID and streams into the same
chat. It preserves unsent text and attachments. A fake executable integration test verifies the actual
Codex resume arguments, native ID, prompt and streamed response; rejection tests preserve history.

`cargo test --all-targets -j4`: 81 passed. `npm run check`: zero errors/warnings. Production build,
ad-hoc codesign verification, DMG checksum and installed-bundle verification passed. The new local and
embedded remote attachment fixtures verify bytes, limits, permissions and symlink rejection before
child creation. Git/deletion fixtures cover nested roots, literal paths, renames, binary/truncated
output, transport errors, exact session metadata and pre-unlink checks. No user session files were
removed during validation.

Installed `/Applications/Monitter.app` from `debug-evMt5c/Monitter.app`, build
`2026-09-10T08:12:30.197Z`, executable SHA-256
`056ed4b19a87fd54015dd8daddce1975c93b818edb0e5392ececc4fe61d79553`.
The previous app and state are backed up. All existing agent, host, chat and message IDs/content were
preserved; messages migrate with empty attachment arrays. The user has resumed using the app, so later
state hashes reflect new activity. Native screenshot confirms the updated app with two independently
scrolling panes, seamless tabs, Resume and attachment controls. See `verification/ui-install-evidence.json`.

The old iCloud blocker is resolved: both exact offloaded skill files were read successfully. A real
Mac-to-Mira run passed discovery, one delegation, one peer inbox message, exact result markers and native
session resume with the persisted result. Evidence:
`artifacts/collaboration-live-local-to-mira-1789027824209059000/result.json`.
This proof used GPT-5.6 Luna locally and GPT-5.5 on Mira, preserving saved user agent models and native
configuration. Local GPT-5.5 now returned HTTP 404 despite appearing in the cached catalog; no fallback
was made in the product. The proof script accepts separate coordinator/peer models for host versions.


Local Mac-to-Mac discovery/delegation/inbox/resume also passed with two separate native sessions:
`artifacts/collaboration-live-local-to-local-1789028167705582000/result.json`. The native smoke result
reported persisted completion through a service restart. The verifier now checks exactly one child
`turn.started` and exactly one final PEER_OK acknowledgement, while allowing ordinary commentary
before that final response. An earlier assertion incorrectly treated that commentary as an extra turn;
rechecking the completed artifact with the corrected assertion passed. Local native IDs:
`01a08a63-733d-7402-8f85-7826bd5a963c` and `01a08a63-c43a-7b32-8623-8288c0827663`.

Browser fixtures passed Projects (7), scrolling (8), collaboration (5), controls (12), new attachments
(4), split panes (4), Resume (3), composer startup/Stop (7), and sidebar/Git (5). They exercise real
frontend state transitions with a labelled test-only IPC fixture; they do not claim live model responses.
Native harness proofs are recorded separately above.


## Composer and pane polish — 2026-09-10

The composer model menu uses the live Codex catalog and per-chat model/effort/Fast overrides. New
drafts remain unstarted until first send. Attachments sit at bottom left, and agent output includes
small sender avatars. Resume is icon-only on interrupted/error native chats. Pane tab bars share the
same macOS scale compensation; inactive panes have a persisted toggle and opacity slider. Dividers
paint one pixel while retaining a wider drag target.

Browser fixture checks passed: model/panes (5), Resume (3), split panes (4), attachments (4), controls
(12), scroll behavior (8), and composer startup/Stop (7), with no page errors. The model/pane suite
checks native-scaled header alignment at 80%, 125% and 200%, one-pixel divider geometry, focus dimming,
setting persistence, model/effort/Fast capabilities, reset/errors/running guards, draft preservation
and deferred task creation. These are frontend fixtures; live catalog evidence is separate in
`verification/model-catalog-2026-09-10.md`. `npm run check` reports zero errors and warnings.

Final Rust suite: `cargo test --all-targets -j4 --quiet` — 89 passed, 0 failed, 2 opt-in live checks
ignored in the ordinary suite. Live reader checks ran separately: local Codex 0.154 returned 6 models
(5 Fast-capable); Mira Codex 0.130 returned 5 (none Fast-capable) using its saved executable path.
Older Mira rejects `service_tier="default"`; unsupported Fast overrides are therefore rejected and
the composer leaves them null. No model turns or native auth/config edits were needed. Draft catalogs
resolve project/agent/host folders consistently with task creation; cache keys include provider,
saved host transport/executable identity and working folder. Project browser checks also passed (7).

Installed and restarted `/Applications/Monitter.app` from `debug-G15zJd/Monitter.app`, built
`2026-09-10T08:48:11.149Z` from source commit `57ddb8b`. Codesign and installed
executable SHA-256 `a38bc2c32fd686269ba5f866ba6d96086e0fb18251c8e01bc4e9fab9895e4c13` verified. All existing persisted fields were
preserved (4 agents, 2 hosts, 8 chats, 50 messages), with private state and previous-app backups.
Native UI inspection confirms two aligned tab bars, the live model selector, bottom-left attachment
button, agent avatars, thin divider and dimmed inactive pane. Installation evidence:
`verification/ui-polish-install-evidence.json`.

## Terminal tabs, activity groups and detail tabs — 2026-09-10

Terminal tabs use xterm.js and an owned native PTY, with local/SSH target resolution and ephemeral
session state. Pane moves retain the same terminal. Focus, async mount ownership, UTF-8 decoding,
bounded output draining and retryable close failures are explicitly handled.

Frontend browser fixtures passed tool-family grouping (4), frosted headers/desaturation (2),
Run detail/Timeline (3), existing pane movement and resizing (4), chat scrolling (8), and
composer startup/Stop (7). Tool checks verify Gmail search/read grouping, description updates,
streamed entries preserving an open popup, outside/Escape dismissal, small-window bounds, muted
borderless rows and unchanged raw events. Sidebar checks verify the shared Git status summary,
non-repository omission, process folder scope and expandable diagnostic output.
These fixture checks validate frontend behavior; they do not claim native shell or model execution.

## Font controls and sidebar/composer refinements — 2026-09-10

Source checks pass with zero Svelte errors/warnings. Font settings have independent face and base
size controls; browser fixtures verified saved values, immediate CSS updates and default fallback.
Native tests passed font-settings serialization/size validation and legacy settings defaults.
The installed build recorded in `verification/latest-install.json` predates these refinements.

Browser checks passed channel mention pills, case-insensitive member matching, automatic recipient
selection/removal, preservation of manual recipients and multiline sending. Matching tests reject
partial names, email prefixes, unknown agents and ambiguous short aliases. Attachment fixtures
passed paste/drop previews, attachment-only sends, failure/retry and pane-move preservation.
Native macOS drag-hover highlighting is implemented but still awaits testing in a rebuilt app.

Additional browser checks verified the inline running-agent indicator and reduced-motion behavior;
host-dot menu and sidebar-footer layout control; sidebar toggle alignment with the native traffic-light
centre; the three sidebar view icons; avatar/name expand-collapse with a 75% black hover overlay;
and exclusion of channel internals from sidebar views and the switcher without deleting tasks.
Thread connector geometry was measured at default and enlarged interface fonts: it starts at the
avatar's lower edge, passes through dot centres, and ends at the final dot.

Channel headers now share DM header sizing/blur/padding; a browser geometry check matched the two
and verified editing through the overflow menu. Channel member sidebars use the existing compact
right-side blade. Browser command checks passed `/invite`, `/topic`, `/names`, `/kick` during a running
turn, and `/admin`, including unknown-agent draft retention, preserved channel history and zero harness
dispatch for local commands. Command parser tests cover case preservation, escaped slash messages,
unknown commands and ambiguous agent names. Channel administration remains user-only.

Native channel membership regression tests passed for idempotence, preserved history, administration
form removal, cancellation and suppression of stale replies after removal/rejoin. Existing channel
send code automatically launches fresh tasks or resumes inactive channel tasks with their native
session IDs. The UI permits sending to idle or busy addressed members; busy recipients now use the durable queue described below.


## Busy messages and terminal exit — 2026-09-10

Browser checks in `scripts/ui-busy-queue-smoke.mjs` passed editable busy DM/channel composers,
visible per-recipient queues and removal, persistent queue/steer preference, Stop availability, and
single channel echo. Native queue, membership and model tests passed. Current harness adapters use
FIFO queueing even with steering enabled; live steering is not connected, and the UI states this.

`scripts/ui-terminal-exit-smoke.mjs` passed Ctrl-D through xterm, automatic tab removal after final
output draining, and exactly one backend close. This is a browser fixture check, not a new live SSH
terminal proof. `npm run check` passed with zero errors/warnings after these changes.

## Workspace restore, tab closing and YOLO — 2026-09-10

`ui-workspace-restore-smoke.mjs` passed split ratios, selected pane/chat tabs, composer drafts,
sidebar state, active secondary-pane Cmd-W, fresh terminal restoration with saved host/folder,
and terminal Cmd-W without closing the pane. Restoring the workspace started no agent turns.
The browser fixture clears native sessions on reload, so this checks fresh-shell restoration;
existing-session reuse is implemented through `list_terminals` and awaits native app QA.

`ui-yolo-smoke.mjs` passed default-off, saved Codex selection, Claude availability, reset on harness
change, and disabled unsupported providers. Four native YOLO argument/validation tests passed,
including local/SSH Codex argument construction. Terminal target resolution tests passed saved cwd
and deleted-host rejection. `cargo check --lib` passed the native menu, list and quit-handshake code.
Svelte check passed with zero errors/warnings. Native Cmd-W menu dispatch and orderly quit/save
acknowledgement still need checking in the next rebuilt macOS app; no install/restart was performed.

## Settings tab and channel freeze regression — 2026-09-10

Settings is now a categorized workspace tab. `ui-settings-tab-smoke.mjs` passes singleton shortcut
routing, autosave, visible save errors/recovery, chat draft preservation, category persistence,
edge split/drag, reload restoration, narrow-pane overflow and Cmd-W tab closing. Wide and split-pane
screenshots were inspected. The independent font settings regression passes in the Typography category.

A channel regression reproduced an unresponsive UI: workspace capture called `saveCurrentDraft`,
which assigned a fresh recipient array, retriggering reactive persistence. Capture is now read-only,
with current composer/recipient values written only into its detached snapshot. This also prevents
background capture from dismissing menus. `ui-busy-queue-smoke.mjs` now passes again, and
`ui-channel-workspace-regression.mjs` verifies bounded saves, preserved recipients/unsent text,
restored channel responsiveness, and working Settings/menu clicks. Svelte check: zero errors/warnings.

### Pointer-based sidebar ordering

`node scripts/ui-sidebar-reorder-smoke.mjs` uses real mouse movement (rather than synthetic HTML drag events) to check agent, chat, channel and project ordering, reload persistence, and normal click navigation. Standard and project views retain independent chat order within each owner; sorting never changes a chat's agent or project. Activity remains running-first/recent-first. Presentation order is stored locally as `monitter.sidebar-order.v1`.

### Pointer tab dragging and saved order

`npm run check`, `node scripts/tab-order-test.mjs`, and `node scripts/ui-tab-drag-smoke.mjs` cover mixed tab order, same-pane/embedded-pane reordering, cross-pane insertion, edge splits, Escape cancellation, stable order on activation, and unsent draft/order restoration after reload. The UI regression uses real pointer movement; internal tabs are not native file drags. Tab-bar drops use insertion markers, while pane body edge drops show split overlays. Workspace saves are deferred during a move and restored on a failed destination, keeping an intact pre-move layout on disk. The dashboard tab is icon-only; native-Mac sizing checks compare both sidebar toggle icons and buttons. Native packaged-app verification requires a new build/install.

`node scripts/ui-close-dashboard-smoke.mjs` checks dashboard close controls and Cmd-W: closing an empty split pane collapses its divider; closing only the dashboard in an occupied pane retains the other tabs. Removing the original main pane promotes a surviving pane and preserves its settings/drafts across reload. The last application pane remains available as the dashboard.

`node scripts/ui-sidebar-resize-smoke.mjs` verifies pointer resizing for the main/right sidebar, 230px/260px minimums, the 40vw maximum, reload persistence, and the right sidebar's narrow-pane blade. Available pane space takes priority when it cannot accommodate the preferred minimum. Widths are local UI preferences; arrow keys resize, Home/End choose bounds, and Escape cancels a drag. Dividers retain a one-pixel visual line with a wider invisible hit area.

`node scripts/ui-agent-settings-smoke.mjs` covers Settings → Agents: existing edit/create/avatar entry points route to the singleton Settings tab, agent selection populates the full form, independent edits survive switching agents/categories and moving/reloading the tab, and explicit save/create/discard work. Avatar, harness, host, permissions and collaboration fields reuse the existing editor; the old agent modal is removed. Font catalogue/system-font discovery were discussed separately and are not implemented by this editor change.

New split panes start with an empty New chat / Terminal chooser, with no dashboard tab. `node scripts/ui-empty-panes-smoke.mjs` verifies lazy chat creation, terminal creation, automatic removal of a pane after its final tab moves away, preserved drafts/terminal sessions, and restored empty panes. The dashboard's under-1000px container rule now retains standard 20px padding while removing the content max-width (superseding the earlier zero-padding rule).

`node scripts/ui-tab-expand-smoke.mjs` covers the three-state per-tab expansion cycle in main and embedded panes, unchanged split widths after restoration, preserved unsent chat text and terminal session IDs, and Cmd/Ctrl-0 resetting interface scale to 125%. Expansion hides sibling DOM branches without removing their sessions or changing the saved layout.

`node scripts/ui-sidebar-controls-smoke.mjs` covers fixed icon-only footer controls, the conditional divider under the sidebar heading, the Agent directory category in Settings (no modal), and live on/off accent tinting of user message bubbles. Pane header backgrounds now use 50% opacity with the existing blur/gradient. Layout presets are accessed through the Controls palette rather than the sidebar.

Composer access selection is covered by `ui-model-panes-smoke.mjs` and the native `task_permissions` test: drafts snapshot their own permission mode, idle saved chats update only their task, running/unsupported changes are rejected, and model/effort/Fast choices retain session and draft state. The native Settings serialization/default/round-trip tests cover opt-in message tint migration.

`ui-autoname-smoke.mjs` verifies chat slash/Controls routing, preservation of unsent text through the Controls action, channel membership, terminal session IDs, and no forwarding of the command to the shell. Native `title` tests cover local/SSH argument construction, bounded title cleanup, a supervised subprocess producing large diagnostics, and timeout when a child never reads stdin. All 22 `runner::tests` pass, including existing local/SSH permissions, quoting and cancellation regressions. These are command/subprocess/fixture checks, not a live model response or packaged-app install test. Auto-name currently requires a configured Codex agent and does not guarantee absence of built-in read tools.


### OpenCode directory mismatch diagnosis (2026-09-10)

Installed OpenCode 1.18.30 was exercised against an isolated loopback OpenAI-compatible
stub, with separate HOME/XDG directories, `--pure`, and no real account credentials or
model requests. New and resumed turns in the same directory emitted
`step_start`, `text`, `step_finish` and exited zero. Resuming the same native session
from another directory emitted nothing and required termination after 40 seconds,
matching the real affected chat's saved-folder/native-folder mismatch. Explicit
`--dir` pointing to the original session directory restored both reply output and
normal exit, even with a different process cwd and stale PWD. Probe scripts and
JSONL output are retained under `artifacts/opencode-diagnosis/`.

This isolates the directory failure; it does not validate every installed plugin,
remote host, or live provider. Existing missing transcript entries are not imported
by this test or by changing the run directory.


### Pane chrome and slash suggestions (2026-09-10)

Svelte check reports zero errors/warnings. `ui-tab-expand-smoke.mjs` and
`ui-sidebar-controls-smoke.mjs` pass with the main sidebar toggle in the main tab bar
and right-sidebar controls in chat headers only. A separate browser check covers
channel member toggling and the fullscreen CSS clearance. Native macOS fullscreen
entry/exit has not yet been exercised in an installed build.

`ui-autoname-smoke.mjs` additionally checks that slash suggestions sit above the
composer, preserve its height and position, retain input focus, and still select
commands through Enter. The menu uses the top layer so composer overflow does not
clip it, with an upward reveal respecting reduced-motion preferences.

OpenCode native regressions passed: three export metadata/timeout tests, two adapter
argument/event tests, and the service folder-correction persistence test. These cover
local fake executables and command construction; live SSH resume was not exercised.
