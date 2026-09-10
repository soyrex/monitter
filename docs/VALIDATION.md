# Monitter validation

Validation of the first locally installed macOS build. This document distinguishes real CLI
execution from browser fixtures and records the limits of each check.

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
- Native startup created a private mode-0600 state file at
  `~/Library/Application Support/com.monitter.desktop/state.json`.
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
