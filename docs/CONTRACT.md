# Monitter desktop contract

Tauri 2 / Svelte 5 / TypeScript / Rust. macOS Apple silicon first.
Canonical TS data shapes: `src/lib/types.ts`. Rust serde fields use camelCase.
All timestamps are Unix milliseconds. IDs are UUID strings. Optional task links use null.
No fake conversations, progress, token counts, host connections or model replies in the shipped app.

## Commands (Tauri invoke names and JSON argument keys)

- `load_dev_ui {}` -> `()`. Owner desktop only. The native app connects directly to exact IPv4
  loopback port 18420, requires a compatible Monitter marker, and then navigates the existing main
  WebView to its dedicated Vite bridge path. It does not start a server, launch another backend, or
  accept an arbitrary URL. The remote renderer has no general Tauri IPC.
- `use_packaged_ui {}` -> `()`. Restores the packaged URL captured when the native app launched.
  It is the only custom Tauri command allowed from the dedicated developer bridge origin. Switching
  either direction reloads ephemeral frontend state, while the same native service, stored data,
  active harnesses and task sessions continue running.
- `get_snapshot {}` -> Snapshot
- `get_ui_snapshot { revision?: string }` -> `{ revision: string, snapshot: Snapshot | null }`
  Shared desktop/LAN UI projection. An unchanged launch-scoped revision returns null; changed
  snapshots contain at most 300 recent activity events (60 per task), omit output/log payloads,
  and bound details to 1,000 bytes (300 for error summaries) rather than full diagnostic history.
- `get_task_events { taskId: string, before?: number, limit?: number }` ->
  `{ events: RunEvent[], nextBefore: number | null }`. On-demand diagnostic pages are newest-first;
  `before` is an exclusive per-task array index, not a timestamp. Pages are bounded and any
  truncated detail is explicitly labelled. Each page has at most 100 events and 128 KiB of
  detail. Original diagnostics remain stored locally. The UI refreshes these pages only while
  the timeline is visible; this traffic is independent of ordinary chat refreshes.
- `get_process_metrics {}` -> `{ cpuTimeMs, residentMemoryBytes, sampledAt, rootPid, processes[] }`.
  Read-only native/LAN sample of the Monitter process tree, including live CLI/ACP harnesses and
  their descendants. CPU time is cumulative, including reaped child CPU, so clients can calculate
  interval usage without shared sampling state; resident memory is the current sum for live
  processes. Each live process includes its PID, parent PID, name, start time, own cumulative CPU,
  and resident memory for breakdown views. Unsupported platforms fail visibly instead of returning
  fabricated values.
- `get_task_event_detail { taskId: string, eventId: string, offset?: number, limit?: number }` ->
  `{ chunk: string, nextOffset: number | null, totalBytes: number }`. Full tool detail remains in
  the local store and is read only after its activity row is expanded. Chunks default to 32 KiB
  and are capped at 64 KiB. Task and event IDs must match; offsets are UTF-8 byte boundaries.
- `read_markdown_file { taskId: string, href: string, basePath?: string }` ->
  `{ path: string, title: string, content: string }`. Native desktop only; it is absent from the
  LAN/controller and visitor surfaces. The task must use a local host. It resolves a relative
  `href` from `basePath` (when supplied) or the task folder; absolute paths and `file:///` URLs are
  accepted only when they canonicalize under that same folder. The supplied base must also be a validated Markdown
  file under the task folder. Remote URLs, non-Markdown extensions, non-regular files, escaping
  symlinks, invalid UTF-8, and files over 2 MiB reject visibly. `path` is the canonical local path
  for use as a subsequent base; `title` is the first H1 or the file stem.
- `get_usage_overview { policy?: "cache-only" | "if-stale" | "refresh" }` -> `UsageOverview`.
  Owner desktop/LAN only. It combines the durable, locally observed per-run usage ledger with
  cached subscription allowance sources. A refresh may perform bounded, read-only local CLI
  probes, never sends a model prompt, changes authentication, or exposes account identifiers.
  Codex uses app-server account rate limits; MiniMax uses `mmx quota show --output json
  --non-interactive`, with `current_interval` represented as its five-hour rolling window;
  OpenCode Go asks the local Codex Router credential broker to query its
  provider usage endpoint without exposing the API key to Monitter. Claude uses
  `claude -p "/usage" --output-format json`: the local command reports zero model turns and
  tokens, and Monitter accepts only its successful `local_command: "usage"` result. The
  `Current session` and `Current week (all models)` headlines are account allowance windows;
  the explicitly approximate, machine-local `What's contributing` analysis is discarded.
  Reset text with an omitted year is resolved to the next plausible occurrence in its named
  timezone. Freshness expiry and probe failures remain visible. All allowance timestamps
  are Unix milliseconds.
  The usage widget presents reset timestamps as minute-granularity relative durations by
  default. Activating any reset duration toggles every timestamp in that widget to a localized
  absolute date and time; activating one again returns the whole widget to relative durations.
- `save_host { host: Host }` -> Snapshot (empty id creates)
- `delete_host { id: string }` -> Snapshot (reject referenced/default local host)
- `probe_host { host: Host }` -> ProbeResult (unsaved settings allowed; versions keyed provider)
- `discover_acp_agents { hostId: string }` -> `AcpCandidate[]` for a saved host.
  Owner desktop/LAN only; not a visitor or mobile-controller command. Discovery
  locates reviewed executable names in known/PATH directories without launching
  agents, installing packages, reading credentials, or making model requests.
  Each candidate contains a preset ID/name/description/source URL, native-or-bridge
  label, `{command,args}` launcher and `detected` flag. Detected does not mean
  authenticated or protocol-verified. SSH connection failures reject visibly,
  rather than being reported as an empty list of installed agents.
- `verify_acp_agent { hostId: string, launch: { command: string, args: string[] } }`
  -> `AcpProbeResult` for a saved host. Owner desktop/LAN only, not shared visitors
  or mobile controllers. Explicitly launches the configured executable and sends
  only ACP `initialize`, bounded to 20 seconds. Never authenticates, creates a
  session, prompts a model, or services filesystem/terminal callbacks. Returns
  negotiated version, safe agent name/version and advertised recovery/content
  capabilities; it does not prove login or model-turn functionality.
- `save_agent { agent: Agent }` -> Snapshot (empty id creates; ordinary callers cannot set `internal: true`,
  cannot clear an existing internal flag, and cannot rename or re-enable collaboration on the resident
  Monitter Admin record. Editing the existing admin preserves `internal: true`, the canonical "Monitter
  Admin" name, and `collaborationEnabled: false`, and may safely reset the resident task/session so a
  reconfigured transport replaces the previous one on next use.)
- `delete_agent { id: string, chatHandling?: "archive" | "delete" }` -> Snapshot (reject internal agents). The default
  is `"archive"`. Archive keeps every chat owned by the removed agent under `Archived chats`, stamps each task with the
  removed agent's display name so the LLM (`provider`/`model`) and the human-readable label remain identifiable after the
  agent record is gone, and stops any resident run with an owner-removal-specific message. `"delete"` additionally
  permanently removes those chats (including verified native session files where possible), dropping the task, its
  transcript, queued messages, pending approvals, pending collaborations, activity events, subagent sessions, the per-task
  host snapshot and the durable usage ledger for the task. Pending queued follow-ups are failed and pending approvals
  expired under the same lock for either mode.
- `save_project { project: Project }` -> Snapshot (empty id creates)
- `delete_project { id: string }` -> Snapshot (unassign chats; preserve their history and runtime)
- `set_task_project { taskId: string, projectId: string | null }` -> Snapshot
- `create_task { input: CreateTaskInput }` -> Task (rejects the internal Monitter Admin agent as a chat recipient)
- `get_model_catalog { target: { taskId?: string, agentId?: string, projectId?: string | null } }` -> ModelCatalog
- `set_task_model_settings { taskId: string, settings: ModelSettings }` -> Snapshot
  Codex tasks may be running: the new model, reasoning effort, and Fast mode are written to the
  task snapshot and applied on the next `turn/start`. Other harnesses keep the existing
  "reject while running" rule because their model/sandbox flags are baked into the launch
  command and need a process restart to apply. Archived tasks still reject.
- `set_task_sandbox { taskId: string, sandbox: Sandbox }` -> Snapshot
  Codex tasks may be running: the new sandbox is written to the task snapshot and applied to
  the next `turn/start` as a fresh `approvalPolicy` + `sandboxPolicy`. Other harnesses and
  archived tasks still reject.
- `rename_task { id: string, title: string }` -> Snapshot
- `autoname { target: { taskId?: string, channelId?: string, terminalId?: string, content?: string } }` -> Snapshot (routed through the resident Monitter Admin turn; see Internal agent)
- `set_task_archived { taskId: string, archived: boolean }` -> Snapshot (reject running; preserve all history)
- `get_task_goal { taskId: string }` -> Goal | null (read-only Codex app-server lookup; version-dependent)
- `get_subagent_transcript { taskId: string, subagentId: string }` -> `SubagentTranscriptEntry[]`.
  Read-only owner desktop/LAN lookup for a subagent reachable from that task's delegation tree.
  For a native Codex child thread it uses `thread/read { includeTurns: true }` without resuming
  the child or starting a turn, and normalizes user/assistant messages, reasoning summaries, and
  tool activity; raw rollout records are never returned to the renderer. For a native ACP subagent
  (`source: "acp"`) there is no separately re-queryable thread, so this instead returns the entries
  already captured inline into `Snapshot.subagentTranscripts` while the subagent streamed.
- `clear_task_goal { taskId: string }` -> void (Codex only; clears the native persisted goal without
  resuming the thread, starting a turn, changing its transcript, or completing the goal)
- `delete_task { id: string }` -> Snapshot (archived only; reject running; preserve native CLI history)
- `send_message { taskId: string, text: string, attachmentIds?: string[] }` -> Snapshot (starts asynchronously)
- `clear_task_context { taskId: string }` -> Snapshot (idle, unarchived tasks with no queued messages or pending requests only; preserves visible history, clears the native provider session, and appends a `Context Cleared` system boundary)
- `send_message_fast { taskId: string, text: string, attachmentIds?: string[] }` -> `{ accepted: true }`
  Same durable acceptance and execution semantics, without a full-history acknowledgement payload.
  Provider startup/control delivery follows acceptance; failures appear through task state and
  diagnostics. Native snapshot, diagnostic-page and fast-send commands run off the UI thread.
- `cancel_task { taskId: string }` -> Snapshot
- `list_terminals {}` -> `TerminalSession[]`
- `finish_quit {}` completes a native quit only after `monitter-before-quit` lets the frontend save workspace state
- `edit_queued_message { id: string, text: string }` -> Snapshot (queued/error only; preserves attachments, recipient, position and status; never retries automatically)
- `cancel_queued_message { id: string }` -> Snapshot (removes only an unsent queued/error message)
- `save_settings { settings: Settings }` -> Snapshot
- `save_channel { channel: Channel }` -> Snapshot (empty id creates; preserve existing messages)
- `set_channel_membership { channelId: string, agentId: string, member: boolean }` -> Snapshot
  validates both records and is idempotent. Removing an agent preserves channel and native history,
  blocks any buffered reply from mirroring into the channel, and cancels that agent's running
  channel task tree through the ordinary cancellation path before a new turn can be sent.
- `send_channel_message { channelId: string, text: string, agentIds: string[], attachmentIds?: string[] }` -> Snapshot
  Explicit selected/mentioned recipients only. Each recipient has a dedicated task under channelId;
  initial/follow-up prompt includes recent channel context. Final replies mirror into channel messages.
- `send_channel_message_fast { channelId: string, text: string, agentIds: string[], attachmentIds?: string[] }`
  -> `{ accepted: true }`, using the same durable channel delivery path.
- `resume_task { taskId: string }` -> Snapshot (continue the existing native session asynchronously in this chat)
- `resolve_approval { approvalId: string, decision: 'approve_once' | 'approve_session' | 'approve_always' | 'deny' }` -> Snapshot
- `revoke_approval_rule { ruleId: string }` -> Snapshot

Event `monitter:changed` payload `{ taskId?: string }` tells UI to reload snapshot (debounce <=150ms).
The backend is authoritative; listen before initial snapshot. Errors reject with a readable string.
Frontend may show a labelled browser design preview when Tauri isn't available, but never simulate an agent reply.

## MCP, plugins and managed skills

Settings → MCP & Plugins manages optional MCP servers and portable Markdown skill
instructions. `get_extension_config {}` and `save_extension_config {config}` return
`ExtensionConfig` and are native-desktop-only commands. They are deliberately absent
from the LAN invoke allowlist and encrypted controller/visitor command surfaces.
Configuration is not a `Snapshot` or `Settings` field. It is stored separately in a
private, atomically replaced `extensions.json` file; environment values and HTTP
headers are not an encrypted credential vault and must not enter workspace exports,
diagnostics, browser storage, command previews, or shared snapshots.
Reads include an opaque revision. Saves from an older settings pane are rejected
instead of overwriting newer edits; the UI retains the unsaved draft.

Internal Monitter Admin prompts, replies, and per-request IDs are similarly
runtime-only and must never enter workspace exports, diagnostics, browser
storage, command previews, shared snapshots, compact LAN UI projections, or
visitor / mobile-controller replies. Only the durable target mutation (for
example a renamed task, channel, or terminal) is reported to the client.

Each MCP server has an ID, display name, enabled flag, exact agent IDs, and either a
stdio command/argument/environment configuration or HTTP URL/headers. Each managed
skill has an ID, name, description, enabled flag, exact agent IDs, an optional
`allAgents` flag, optional `sourceUrl` provenance, and Markdown content.
`allAgents` includes current and future user agents, excluding the internal Monitter
Admin. It defaults to false for existing data. An empty selection grants nothing
unless `allAgents` is true. Saving configuration never starts a server,
installs a package, calls a model, changes native CLI authentication, or approves a tool.
Changed MCP entries revoke remembered approvals for affected agents so a replacement
server cannot inherit the previous server's tool grants.
Remembered rules also include the MCP configuration digest captured by the actual
resident run, so an old process cannot create a grant for a replacement configuration.

Managed skills are portable instructions delivered to the assigned harness, not native
plugin bundles. Importing a Markdown file does not install accompanying scripts or
assets. Native plugins remain harness-specific and are explicitly labelled unsupported
in this manager. MCP changes apply to a subsequent harness launch; an existing resident
session is not silently restarted, and no message is replayed to apply configuration.
Unsupported harness/transport combinations must fail visibly rather than silently
running without an assigned MCP server. Monitter's built-in collaboration server remains
separate and keeps its existing exact tool scope.

Current managed MCP support is local Codex app-server (stdio/HTTP), local Claude
(stdio/HTTP), and local ACP (stdio; HTTP only when advertised at initialization).
SSH and the legacy OpenCode/Hermes adapters reject assigned managed MCP servers.
Use OpenCode through ACP for this feature. Skill text is added to execution context
without changing the stored user message; this does not erase instructions already
present in a native session's history. Codex receives required startup and prompted
tool approval for managed servers, never the built-in collaboration allowlist.

### Shared skill installation through the built-in MCP

A running agent with collaboration enabled can discover this workflow through
`skills_help`, inspect metadata through `list_shared_skills`, and call
`install_shared_skill {url, name?}` when the user requests an installation.
The tool accepts public HTTPS Markdown files, raw GitHub URLs, GitHub blob/tree
URLs, and repository URLs with a root `SKILL.md`. For repositories with multiple
skills, provide the specific skill file or directory. Generic HTML installation
pages must first be resolved to their actual Markdown skill URL; downloaded
instructions are not executed to discover or install dependencies.

Example: `install_shared_skill {"url":"https://github.com/owner/repo/tree/main/skills/shipping"}`.

The download is bounded to 128 KiB, uses public HTTPS addresses pinned after DNS
validation, and does not follow redirects or use credentials. No Git clone, shell
installer, package manager, bundled scripts or assets are run or installed.
The response explicitly states this Markdown-only limitation. Remote instructions
are untrusted content and cannot authorize additional actions.

Installation adds one enabled `allAgents` skill to the latest private extension
configuration under the same write lock used by Settings. It does not return MCP
secrets or skill contents. Exact repeat installs are idempotent; source/name
conflicts, including disabled or edited skills, fail without replacing existing
configuration. Stale Settings revisions remain rejected. Settings → MCP & Plugins
can review, edit, disable, remove or narrow the assignment afterward.

The caller must still be an active, noninternal collaboration-enabled agent after
the download completes. Codex installation calls use a per-tool `prompt` policy;
Claude does not put installation on its built-in automatic tool allowlist. Other
harnesses retain their native permission handling. Read-only help/list tools are
available alongside the existing collaboration tools. No new LAN/controller
configuration command is exposed.

Saved instructions apply at the next harness launch. Existing resident sessions
are not restarted or replayed; launch a new task/session to consume the skill.

### Follow-up: managed MCP over SSH (not implemented)

- Forward per-agent configuration through the encrypted SSH session to the remote
  harness using invocation/session-scoped configuration. Preserve remote CLI auth
  and existing configuration; do not overwrite global files.
- HTTP servers must be reachable from the remote host. A URL pointing at localhost
  refers to that host, not the desktop Mac; do not silently reinterpret it.
- Stdio servers require executable and dependency checks on the remote host, with
  remote path resolution rather than copying local absolute paths. Do not install
  missing dependencies automatically.
- Mac-only HTTP endpoints need an explicitly configured, owned reverse tunnel bound
  to remote loopback, with bounded startup and cleanup. A Mac-local stdio server
  additionally needs a protocol bridge; a TCP tunnel alone cannot expose it.
- Require explicit host-scoped consent before forwarding credentials. Keep secrets
  off command arguments and logs; use the encrypted channel and private temporary
  configuration when necessary, cleaning it up after the session.
- Respect each harness's MCP capabilities and existing approval scopes. Surface
  executable, authentication, reachability and tunnel failures; never silently omit
  assigned servers or replay a turn to recover configuration.
- Verify direct remote HTTP, remote stdio, optional reverse forwarding, cleanup and
  secret redaction with SSH integration tests before enabling support.
- Portable Markdown skills can travel in execution context without remote
  installation. Bundled scripts/assets would require a separate transfer design.

## Busy messages

When a reader scrolls above a transcript's latest entry, the frontend holds a data snapshot of that transcript. New messages and updates to streamed text or tool activity continue in the backend, but are not rendered into the held view. The down-arrow indicates once updates are waiting and releases the latest view when activated or when the reader manually returns to the bottom. Sending a message and switching chats also release the held snapshot. Approval controls and failure notices remain live outside the held transcript.

Chat sends render a frontend-local outgoing message immediately and clear the captured composer
text/attachments while the command is in flight. Sending, Sent and Not confirmed distinguish
transport progress from agent execution; Sent means Monitter accepted the message, not that the
agent read or completed it. New typing is never cleared by a delayed acknowledgement.
Snapshot reconciliation replaces the local echo with the persisted message or queue entry without
duplicating it. A transport failure retains the outgoing content and offers explicit retry; retry
first refreshes the snapshot and will not resend while acceptance cannot be checked. These delivery
indicators are window-local UI state, not new provider messages or simulated assistant output.
The per-pane outbox is retained in sessionStorage where available; reloading changes an in-flight
item to Not confirmed and never automatically replays it.

`Settings.busyMessageMode` defaults to `queue`; the Settings toggle selects the default queue-versus-steer
behaviour for busy agents. `Snapshot.queuedMessages` persists FIFO records per task, including the
original text, attachment IDs, optional channel, status and error. A busy direct chat or selected busy
channel recipient queues its follow-up; idle recipients still start immediately, and a channel user
message is recorded once at submission rather than again when a recipient drains. With `steer`, a live
local Codex app-server task sends `turn/steer` using its current thread and turn IDs. Monitter retains a
visible `sending` record until Codex acknowledges the matching turn, then persists it as a user message;
an unavailable, stale, or unsteerable turn returns the record to FIFO with a visible fallback event.
Other CLI adapters safely queue. The next queued item starts only after its owned native run releases. A
restart changes an uncertain `sending` item to `error` and never replays it automatically. Cancelling, kicking,
archiving or deleting prevents queued follow-ups from launching; task deletion removes their records.

## Agent identity

`Agent.avatar` is a nullable embedded PNG/JPEG/WebP Base64 data URL. Missing avatars migrate to null.
The frontend accepts files up to 2 MiB; the backend rejects remote URLs, SVG, malformed Base64 and
serialized values above 3 MiB. Images stay in the app's private local state; selecting or removing
an avatar does not alter the agent's native session. Missing avatars use a deterministic icon from
the bundled icon set on the active theme accent colour.

An existing chat header contains its subject, Resume and the task overflow menu, plus Stop while
running. Folder, harness, model, host, project and permissions appear at the top of run detail with
Agent identity. The lowercase monitter menu contains Preferences, Hosts and Agent directory; no separate host row
is shown in the sidebar.

## Appearance and conversation preferences

Surface tint intensity is a client-local appearance preference stored in
`monitter.appearance.surface-tint.v1`, not a backend Settings field. The slider
ranges from 0–50 in integer percentages, defaults to 5, and previews on input.
Dark mode uses the selected intensity; light mode uses half. System theme follows
the same rule. Missing/unavailable storage uses the default and failed storage
writes do not prevent live preview. No backend rebuild is required for this control.

Border and divider opacity is a client-local appearance preference stored in
`monitter.appearance.border-opacity.v1`. It ranges from 0–100 percent and defaults
to 100, preserving the existing line strength. It changes the shared structural
line colour without fading text or content, and does not require a backend rebuild.

Decorative interface motion is client-local in `monitter.appearance.motion.v1`:
`system` (default), `subtle`, or `off`. The operating-system reduced-motion
preference is a hard limit in both System and Subtle modes. Off or OS reduction
disables spatial transitions and cancels active decorative animations without
changing navigation, focus, backend operations or progress semantics. Motion
changes never persist through backend Settings; clients can differ independently.

Base sizes `interfaceFontSize`, `chatFontSize`, and `terminalFontSize` are integer pixels (8–32), defaulting to 14, 13, and 14 respectively. Interface typography retains its relative hierarchy; native interface zoom scales all three exactly once.

Settings include optional `interfaceFont`, `chatFont`, and `terminalFont` family names. Empty or missing values use IBM Plex Sans for interface/chat and IBM Plex Mono for terminals. Installed custom family names are supported with fallback fonts; changes apply to existing terminals without restarting sessions.

Terminal colour themes are client-local and stored in `monitter.appearance.terminal-theme.v1` so native,
browser, and remote views can differ without a backend rebuild. The choices are Monitter, Catppuccin
Mocha, Dracula, Gruvbox Dark, Molokai, Nord, One Dark, Solarized Dark, Wombat, and XTerm. Changing
the choice applies the full foreground, background, cursor, selection, and 16-colour ANSI palette to
all mounted terminals immediately; new terminals inherit it.

The whole-app palette pair and contrast boost are client-local in `monitter.appearance.app-theme-pair.v2`. Light and dark
appearance each have an independent rich selector with ten paired families: Monitter, Catppuccin,
Dracula, Gruvbox, Nord, One, Solarized, Tokyo Night, Wombat, and XTerm. System appearance swaps the
selected light/dark palettes through CSS, including their surfaces, text, line, code, and accent colours,
without a reload. A 0–100 contrast slider sits directly below the palette selectors and defaults to 0,
preserving each preset. Increasing it progressively moves dark-theme surfaces toward black and text toward
white, with the inverse treatment for light themes; accent hues remain unchanged. The System/Light/Dark appearance mode sits above the palette selectors. The collapsed
Advanced theme controls let one optional accent, tint intensity, and border opacity apply to both; clearing the accent override
restores each palette's own accent. A legacy single-preset selection is migrated once when no paired
selection exists. Density, motion, scale, tabs, pane controls, and terminal themes remain independent.
No backend rebuild is required.

The browser `theme-color` meta value and the document background follow the effective sidebar colour,
including the active light or dark palette, shared accent override, and surface tint. System mode updates
both when the operating-system colour scheme changes. The calculated light/dark pair and last appearance
mode are cached in `monitter.appearance.browser-colours.v1` and `monitter.appearance.mode.v1` so the next
page load can set the document colour before Safari captures its surrounding chrome. This covers both
older Safari theme-colour behavior and Safari 26's document-background colour extension.

Settings include `accent`, `theme`, legacy `interfaceScale` (integer percent, 80–200, default 125),
`showToolActivity` and `showReasoningSummaries` (default true), and `sendWithEnter` (default false).
The legacy shared scale seeds client-local `desktop` and `mobile` viewer preferences once. Current
values live at `monitter.interface-scale.v1:desktop` and `monitter.interface-scale.v1:mobile`; they
follow the viewer type across connection changes and are never written back through shared settings.
The legacy shared `sidebarView` field remains readable for one-time migration only; current sidebar
selection is client presentation state and is never written back through shared settings.
`interfaceDensity` is `tight`, `normal`, or `spacious`, defaulting to `normal`. It changes
workspace chrome rather than conversation typography: tab bars, compact tab selectors, pane headers,
sidebar rows and their associated controls share density variables. Normal is lower than the former
fixed tab bar; mobile touch targets retain their accessible minimum sizes in every density.
`tabStyle` is `classic` or `modern`, defaulting to `classic`; missing values from existing saved
workspaces resolve to `classic`. It is a shared desktop/LAN appearance preference.
Modern tabs use straight edges and fill a bar four pixels shorter than Classic's base bar height,
with a 32px minimum. The left-sidebar view tabs and right-sidebar detail tabs follow the same style;
the left view tabs sit below the Monitter header and the detail tabs below the unified chat header.
Their rows are 34px in Classic and 32px in Modern, with horizontal overflow confined to the tab
lists. Empty workspace tab-strip space remains a native window-drag region; tab buttons
keep their separate reorder/move behavior.
Solid accent-colour controls choose black or white foreground icons and labels from the higher WCAG
contrast ratio. The paired mobile interface uses the same computed foreground.
Settings tabs display their active section as `Setting: Appearance`, `Setting: Typography`, etc.
The horizontal tab, compact selector and workspace sidebar use the same section-aware title.
Missing new fields receive these defaults when older saved workspaces load. Native WebView zoom
scales the active desktop or mobile viewer interface; window controls keep their native size and
reserved header space. Browsers apply the active viewer preference as layout zoom with inverse
viewport dimensions, so text, controls, spacing, tabs, and panes scale together without overflow.
Crossing the 760px mobile-layout breakpoint switches buckets without overwriting either value; the
dedicated paired-phone route always uses the mobile bucket. Cmd/Ctrl+plus (including Cmd+=) and
Cmd/Ctrl+minus adjust the active viewer by five percentage points within 80–200%. They do not trigger
the browser's own page-zoom command.
Cmd+Option+= on macOS and Ctrl+Alt+= on Windows/Linux balance every open pane to the same area while
preserving the current split arrangement. The same action is available as Balance panes in Controls.
Cmd/Ctrl+Shift+arrow moves the selected tab into the pane in that direction. If no pane exists there,
it creates a split and keeps the empty source pane. Cmd/Ctrl+Option/Alt+Left/Right selects the adjacent
tab, crossing a pane boundary when needed; Up/Down focuses the pane above or below.
Activity toggles filter both inline blocks and run detail without deleting captured events.
Cancelling a running task writes a durable system transcript record, `You cancelled this run.`,
and a matching diagnostic status event. The transcript record appears inline in the owning
desktop/LAN and paired-mobile chat with a stop icon and right-aligned timestamp; it is not an
agent reply. Other status diagnostics remain in Run detail, and visitor sharing continues to
exclude activity events.
Only reasoning summaries emitted by the harness can be rendered; missing reasoning is not fabricated.
Consecutive reasoning events for the same chat render inside one expandable reasoning bubble. Messages,
tool activity, approvals, and task boundaries split that group; stored events remain unchanged.
Enter-to-send applies to tasks and channels. Shift+Enter always inserts a newline; IME composition
never submits a message. With Enter-to-send off, Cmd/Ctrl+Enter submits instead.

Open task tabs live in the window header. Closing a tab preserves the task and its draft; reopening
from the sidebar restores it. Tab selection is window UI state, separate from persisted task history.

New chat opens a local draft tab with message, agent and project choices, starter suggestions,
and optional title/native session attachment. No task or harness process is created before the
first message is sent. Draft fields, unsent text and attachment references are saved with the desktop
workspace and survive navigation and restart. Closed draft tabs remain discoverable through Cmd-K.
Submitting that first message immediately replaces the setup form with the standard chat layout and
shows the optimistic user message there while task creation and harness startup continue.
A failed send after task creation reuses that task on retry. A delayed send
completion must not replace the conversation the user has navigated to.

## Projects and sidebar views

Projects are independent of agents. `Project` stores `id`, `name`, `description` and optional
`workspaces: [{ hostId, cwd }]`. Missing projects/workspaces default to empty arrays; missing
`Task.projectId` and `CreateTaskInput.projectId` default to null. A project may contain chats from
multiple agents and hosts. Each workspace must reference a known, unique host and have a nonempty
folder; blank fields in the form mean no mapping for that host.

When the first draft message creates a task, its project folder for the selected agent's host
overrides the agent folder. Without a mapping, the existing agent/host folder fallback applies.
Changing a project's folders affects future chats. Moving an existing chat changes only its
project link: cwd, host, native session, running turn, messages and activity timestamps stay intact.
Deleting a project unassigns its chats, including archived/running chats, without deleting them.
A host referenced by a project workspace cannot be deleted until that mapping is removed.

The sidebar offers icon tabs for Agents (agent groups), Projects (collapsible project folders plus
unassigned chats), and Activity (running first, then most recent), in that order. These tabs replace
the matching content headings. Their compact heading-scale labels tighten with the sidebar width;
narrow widths retain all three tab icons while hiding their visible labels. The selection is stored locally and independently
for native desktop, ordinary web and mobile clients; changing one must not update a shared snapshot or
alter another client's view. Switching updates the visible list synchronously without waiting for a
backend settings write. Each browser tab retains its own choice across reloads using session storage;
local storage supplies the default for a new client window. Split panes share their window's selection.
Channels remain available in each view. Archived chats stay out of these
ordinary lists and remain discoverable through Cmd-K. Projects participate in the switcher; controls
include new project and sidebar view selection.

## Runtime and persistence

ACP is a generic transport (`provider: "acp"`), not an agent-brand enum. Its
optional `Agent.acp` / `Task.acp` launcher contains an executable `command` and
an exact string-array `args`, never a shell command. Older records omit it.
New ACP tasks copy the agent's launcher; editing the agent does not retarget an
existing chat. Non-ACP tasks do not carry an ACP launcher. Session ownership
distinguishes ACP launch configurations. ACP normally uses `harness-configured` permissions.
An explicit `yolo` selection requests the live session's advertised `bypassPermissions` mode;
if that exact option is absent the launch fails visibly. ACP remains a transport, not an OS sandbox.

Agent settings offer searchable presets and a custom ACP launcher. The catalog
is convenience metadata, not a restriction on which compatible executables can
be used. Bridges (including Pi ACP) are labelled separately from native ACP.
Pi's native `--mode rpc` must never be treated as ACP. Existing OpenCode tasks
remain on their prior adapter; OpenCode ACP is selected explicitly for new chats.
Launcher paths/arguments remain owner-only and are omitted from shared-visitor
agent and task projections.

Local Codex chats use one owned `codex app-server` process per task over private stdin/stdout
JSON-RPC. Initialize once, create/resume the saved native thread, and start each user turn on that
connection. No HTTP/WebSocket listener is exposed. SSH Codex retains `exec --json` and
`exec resume --json <id>` and does not advertise interactive approvals.
Codex app-server bounds initialize to 20 seconds, thread start/resume to 150 seconds, and each
turn start to 20 seconds with independent clocks; notifications never extend a pending request.
Resume requests exclude stored native turns because Monitter preserves the local transcript.
Resume is explicit by native session ID. Users may attach an existing idle native session by ID; old
transcript import and live takeover of another Desktop/TUI process are not implied.
Local Codex replies use item text deltas and authoritative completed items. The same durable
message is updated rather than appending a message per token; `Message.streamStatus` is optionally
`streaming`, `complete`, or `interrupted`. Legacy messages omit this field. Partial text survives
restart as interrupted. Only completed replies mirror into channels or trigger peer routing.
`Message.phase` optionally preserves Codex `commentary` or `final_answer`. Final-answer messages use
a subtle green, left-tailed chat bubble in desktop, LAN, shared and paired-mobile task transcripts;
commentary, legacy records and other harnesses keep the ordinary assistant presentation.
Codex `userMessage` and `agentMessage` lifecycle envelopes are transcript transport metadata, not
tool activity: new envelopes are not persisted as RunEvents, and legacy envelope events are hidden
from Timeline while their ordinary user or assistant messages remain visible in chat.
Every mutation is scoped to the owned process and native turn; stale events cannot update a new turn.
Use real CLI account/config; do not copy auth or change global config. Resolve local CLI paths even
when launched by Finder with limited PATH (user local bin, Homebrew, standard dirs).
Claude uses one resident `--print --input-format stream-json --output-format stream-json --verbose`
process per local desktop chat. It receives subsequent user turns and permission decisions over the
same JSON-lines stdin, retaining its native context; `--resume` bootstraps controlled process recovery,
restoration after idle retirement, or an explicit handoff. OpenCode uses
`run --format json --thinking --dir <folder>` and `--session`. Before resuming OpenCode,
a bounded read-only `export <session>` lookup verifies the native ID and restores the original
session folder into the task snapshot, before collaboration setup. This prevents inherited PWD
or a mismatched attachment folder from silently losing the CLI event stream. Export contents
are not imported into the chat or logged. Hermes runs the installed TUI gateway with a Python
bridge: session.create/resume, length-framed multiline prompt.submit, durable stored_session_id, normalized JSONL events,
explicit denial of interactive requests, and owned child cleanup. An explicit Hermes `provider/model`
selection is passed as separate session-scoped provider and model fields; unqualified models retain
Hermes' configured provider. Never silently substitute a harness.
Provider stderr is diagnostic-only: routine trace, debug, info and warning output is discarded.
At most one concise actionable provider error may appear in a chat's activity, while process/event
failures remain visible through their normal task status.

A service-owned collector checks resident runtimes every 60 seconds, filtering by idle age before inspecting processes. Eligible local macOS
Codex app-server, Claude stream-json and resumable ACP processes retire after more than five
minutes continuously idle following a durably completed turn. Task history, settings and native
session IDs stay saved. The next accepted message silently starts the same native session and
submits that message once; it may take longer because the process must start again. Retirement
never submits a continuation prompt and never replays a failed turn. One-shot providers already
exit after each turn. SSH runtimes remain resident because local process inspection cannot prove
that remote background work has ended.

Active turns, pending approvals/input, queued messages, protocol requests, and unidentified live
descendants prevent collection. A saved session and negotiated cold-resume support are required.
The collector verifies process identities against the infrastructure process tree captured before
the first prompt and refreshed only after tool-free completion, until any tool work is observed. It closes owned pipes,
and uses bounded termination of the owned process group and verified captured helpers, including
helpers that detach or reparent. PID and start time are checked before signalling those helpers.
The old native-session writer remains
reserved until teardown completes. Accepted sends wait for that reservation; cancellation
invalidates their dispatch receipt. Successful retirement adds no visible chat status or messages;
teardown or native-restoration failures remain visible. Process-scoped approval grants expire at
retirement; remembered rules keep their existing scope. See `IDLE-RUNTIME-RETIREMENT.md`.

An owned ACP stdio transport that closes after a native session has been saved is repaired once,
with a fixed 60-second runtime initialize deadline to accommodate cold provider/plugin startup,
without client intervention: Monitter replaces the dead process, repeats the bounded initialize and
`session/load` or `session/resume` handshake advertised by that ACP agent, and leaves the replacement
resident for the next desktop, web or Android send. A successful idle repair is silent. If a prompt
was outstanding when the pipe closed, its delivery is ambiguous even when the provider's saved log
does not contain it. Monitter marks that turn interrupted, expires its approvals, preserves its user
message and partial output, reports the ambiguity once, and reloads the session without replaying the
prompt. Enqueuing a frame to Monitter's stdin writer is not proof that the agent received it, so this
path never guesses based on a transcript or blindly retries an accepted turn.

Each transport-loss episode permits one replacement only. A replacement does not earn another
reconnect attempt until it has completed a subsequent real prompt; this prevents a crashing agent
from forming an idle reload loop. Failure of the replacement initialize or session recovery is
terminal rather than recursive. Cancellation, shutdown, an invalid launcher or
configuration, permanent provider/authentication rejection, and an ACP agent that does not advertise
recovery also fail or interrupt normally and are not silently retried. Recovery retains the exact
task, launcher, host, working folder, native session and single-writer claim; an old process finishing
cannot evict its replacement. Recovery diagnostics are bounded and exclude command arguments,
environment values, credentials and raw provider frames.

Host.claudePath defaults to an empty string for older stored host snapshots. Task.archived defaults
false; archiving preserves messages/events/native IDs and hides the chat from ordinary navigation.
Cmd-K explicitly restores archived chats. Channel sends start a new task instead of reusing an
archived one. Permanent deletion is available only after archiving. The confirmation preserves CLI history by default; an optional switch requests verified native session file cleanup as described below.

Persist configuration and transcript in the private Tauri app data directory using
`state.sqlite3`. SQLite stores each Snapshot collection record, task-host map entry,
attachment registry entry, and usage sample as an individual row; collection position
preserves array ordering and collection/task/creation indexes support bounded reads. It
uses WAL with `synchronous=FULL`, a serialized transaction writer, and row-level
insert/update/delete/reorder diffs. Retained records at the same position are not
rewritten. Readers continue to see the last committed immutable state while a candidate
is prepared and committed; AppKit keyboard handling never reads or clones transcript
state. A failed or ambiguous database commit poisons further writes until reopen rather
than publishing a stale candidate.

Opening an existing database is strict: it must be a regular non-symlink file with the
expected SQLite application ID, schema version, tables and indexes. WAL/SHM sidecars
must be absent or regular non-symlink files; database and created sidecars are private.
Malformed row payloads, mismatched row/payload IDs, malformed positions, unexpected
schema, missing database after a guard, or migration-source changes fail closed rather
than silently starting from empty state.

The first SQLite migration records a durable intent containing exact hashes of every
legacy source, writes exact private source backups, imports and verifies SQLite, WAL
checkpoints the migration database, atomically installs it, and finally replaces
`state.json` with an explicit SQLite downgrade guard. The guard makes old binaries fail
to decode state rather than silently ignoring acknowledged SQLite updates. Interrupted
migrations resume only when the intent, source hashes, database integrity, and imported
state hash all agree; otherwise they preserve files and report an error. The new Store
holds a private lease for its lifetime so two current versions do not write the same
directory concurrently.

An already-running old binary does not honor that lease. It must be verified stopped
before migration: otherwise it can write legacy state after final verification and
before the guard replacement, which would lose that late legacy update. After SQLite
writes begin, downgrade requires an explicit, tested legacy-format export while no
writer is active, or restoration of a complete pre-upgrade backup; a SQLite checkpoint
is not a legacy export. Never run old and new binaries concurrently against the same
data directory.
Recover formerly running tasks as interrupted after restart. One active turn per task and provider/host/native-session key.
Store task host/cwd/provider/model/sandbox as a snapshot when created; agent edits affect new tasks.
An idle, unarchived task may change its saved sandbox through `set_task_sandbox`; its next native
launch or explicit resume uses that saved mode. A running process keeps its launch-time mode.
Codex defaults to read-only with explicit workspace-write selection. `yolo` is an explicit,
per-agent choice, captured in each newly created task's snapshot and off by default. Codex invokes
`--dangerously-bypass-approvals-and-sandbox`; Claude invokes `--dangerously-skip-permissions`.
OpenCode and Hermes reject `yolo`: OpenCode's `--auto` still respects explicit denials, and the
Hermes bridge has no verified per-invocation bypass. Other providers require `harness-configured`;
do not describe their host permission rules as an OS sandbox.
Approval requests are durable task records, not generic tool activity. A request contains the provider,
provider run/request ID, proposed tool/action summary, provider detail, risk label, timestamp and an
explicit one-time approve or deny decision. Pending requests stay pinned above the owning chat's
composer. Resolved decisions appear as compact, timestamped transcript lines linking to their full
records in the right sidebar's Approvals tab, not full cards in the chat. A stopped task, expired response channel, restart or unsupported
provider transport resolves a pending request safely without authorizing work. Never render an approval
denial as a successful `Tool result`, and never render an enabled approval control unless that live
provider run can receive the decision.

Known file-change requests may additionally offer `approve_session`. This grants all later file-change
requests from the same chat and exact live harness process, across turns, while leaving shell commands,
network access and other permission classes prompted separately. The grant is runtime-only: cancellation,
process loss, archive/stop or app restart removes it. Each automatically accepted edit still creates an
auditable approval record, and providers receive only the one-time response for that concrete request.

An eligible exact tool action may also offer `approve_always`. This creates an app-owned, durable and
revocable rule—not a provider permission grant. Rules are scoped to agent, immutable task host
configuration, provider, cwd, sandbox, ACP launcher where applicable, and an exact canonical tool input.
Only non-interaction requests with provider-supplied raw action data are eligible; titles alone,
questions, forms, URLs, missing/oversized input and unknown semantics are never rememberable. Only known
provider-envelope correlation IDs are removed at the top level; IDs inside tool arguments remain semantic.
A matching future request is auditable with its rule ID and `approve_always` decision, but the response sent
to every native harness is always one-shot approve. Rules never survive a scope change, do not replay stale
work after cancellation/restart, and can be revoked from Settings or the Approvals sidebar.

Current interactive approval transports include local Codex app-server, Hermes' local full-duplex gateway and Claude's local
stream-json host protocol. A Claude `can_use_tool` request creates one durable desktop approval and the
chosen one-time allow or deny is returned to that exact control request ID with the provider's original
tool input preserved. SSH Hermes and SSH Claude remain safe-deny/unsupported until their remote
supervisor supports the same persistent full-duplex channel. SSH Codex `exec` and OpenCode `run` do not
show enabled interactive approval buttons. No auto-resubmission after
errors.

The resident Monitter Admin transport is a separate, single-occupant lane that the runtime owns for
its own use (currently `autoname`): the bootstrap migration creates exactly one internal agent named
"Monitter Admin" on `Service::open` and hides it from conversation surfaces while exposing its
configuration in Settings → Agents. The internal task is
created lazily on first use, never archived, and never projected into ordinary chat lists. Repeated
`autoname` calls reuse its resident transport. It follows the same five-minute idle retirement
policy, with an active broker request preventing collection; the next request restores its saved
native session silently. The transport also stops on app quit, on admin
reconfiguration that changes its provider/host/cwd/sandbox (so the next use launches a fresh resident
session), and on transport failure. A restart resumes the saved native session id from the persisted
task; no silent retries are attempted after a failed, timed-out, or interrupted turn, and concurrent
admin requests are rejected with a readable "Monitter Admin is busy with another interface request."
error. The admin's configured provider, model, host, cwd, and sandbox are used as-is: no Spark/Luna/Mini
mini-model selection, no provider fallback, and no automatic retry.

Codex command/file/permission approval decisions remain one-time approve or deny and are returned
to the exact JSON-RPC request. Structured questions and supported MCP forms attach optional `input`
to the durable ApprovalRequest, with `response` retained after submission. `resolve_input
{approvalId,response}` submits validated answers through the owner desktop/LAN bridge; visitors
receive no approval/input records. URL elicitations require an explicit user action; links never
open automatically. Unsupported requests are explicitly declined, never treated as approvals.
Cancellation, process loss and restart expire pending requests; uncertain turns are not replayed.

SSH host port 0 means use the existing SSH config/default (omit `-p`). An empty user/identity path
also preserves the SSH config. Expand remote `~/` relative to the remote home, never the local home.
SSH uses the system client, keys/agent/config, BatchMode, bounded connection timeout and normal host-key
verification. Quote each remote argument using POSIX single quote escaping; prompts always stdin.
When a configured remote CLI is an explicit path, its parent directory is prepended to that owned
process's PATH for probes, turns and read-only catalog/goal helpers. This lets NVM/npm launchers find
their sibling runtime in non-interactive SSH without sourcing shell profiles or changing remote config.
Do not disable StrictHostKeyChecking or expose a public network listener. Killing a local process must not
kill unrelated agents. Cancellation and app quit must clean up owned children as far as transport permits.
No purchase or paid provisioning; existing authorized Codex account usage only.

## Activity and navigation

The document/root never scrolls. Each pane has a bounded independent overflow; short/high-scale
windows also bound headers, composer and activity panels. Cmd-K is the workspace switcher and
Cmd-P is the controls palette. Both preserve drafts, contain keyboard focus and support filtering,
arrow navigation, Enter and Escape. Settings use native checkbox inputs styled/announced as switches.

Activating a chat or channel opens its message pane at the latest message. Incoming content follows
the bottom while the reader is already there. The first upward wheel, touch movement toward older
messages, scrollbar drag, or upward scrolling key cancels any pending automatic follow before the viewport moves; scrolling up therefore
pauses immediately, even inside the normal near-bottom threshold, and shows a keyboard-accessible
down-arrow button to jump back. Successful sends reveal the submitted message
in the same active conversation. A delayed send acknowledgement cannot scroll a different chat.
Content/pane resizing preserves bottom-following, including late image/font loads and tool expansion.
Downward and horizontal gestures at the bottom preserve following, including trackpad momentum.

RunEvent.kind additionally accepts `computer`, `goal`, and `log`. Computer detail contains
`{id,phase:"started"|"completed",tool,summary}`. Only start records in the current user turn of a
running owned task activate the compact panel. Matching completion or terminal state clears it;
historical tool records cannot reactivate control. Stop cancels the task's owned process group.
Codex `ContextCompaction` tool detail preserves its native `id` plus
`monitterPhase:"started"|"completed"`. The UI may show a duration only for a matching pair;
an interrupted or legacy lone record remains neutral rather than being described as completed.

`get_task_goal` performs only the Codex app-server initialize/initialized/thread/goal/get handshake,
with bounded cleanup. It neither resumes nor changes a thread. A null result hides the panel; API
errors remain inspectable in run detail. Objective/status/token budget/usage come from the harness.
`clear_task_goal` uses the same bounded app-server lifecycle and sends
`thread/goal/clear { threadId }`. It is available only for Codex tasks with a native session ID;
it does not simulate completion or hide a goal locally. Errors remain visible so the existing goal
card stays available for retry. A native `{ cleared: false }` response is an idempotent success:
there was already no persisted goal for that thread.
Hermes goal notifications have free-form text and appear as the last reported update without
fabricated objective/status/token metrics. OpenCode plans are not presented as native goals.

## Slash menu

Typing `/` opens a filtered, keyboard-accessible menu labelled Monitter commands. These are app
actions: `/new` and `/clear` reset the active chat's provider context while preserving its visible transcript; outside an active chat `/new` opens an independent draft. `/settings` opens preferences, `/project` selects the
chat's project, `/stop` cancels a running task, `/resume` continues the saved native session in this chat, and
`/goal` reads the available Codex goal. Task-specific actions only appear in applicable contexts.
`/autoname` names the current chat or channel from its recent messages. Controls → Auto-name current pane also names an active terminal from a bounded recent-output buffer; it is never written to that shell. Naming routes through the resident Monitter Admin agent (see Internal agent) on its saved provider, model, host, cwd, and sandbox. The admin uses no Spark/Luna/Mini mini-model selection, no provider fallback, and no persisted admin prompt/reply. Concurrent requests are rejected; the request is bounded to 45 seconds; the final target mutation (task title, channel name, or terminal rename) is reported through `monitter:changed` exactly like any other rename.
`/terminal` opens a terminal tab in the current host and folder; `/terminal <command>` runs that shell command in the new interactive tab (for example `/terminal npm build`). The command is written to the PTY after the shell starts, so the tab stays interactive when it finishes.
Selection supports arrows, Enter, Escape and clicking. IME composition does not select an action.

The current CLI transports do not expose a shared native slash-command catalog. Unknown commands
and command arguments such as `/goal objective` produce a visible explanation without sending them
as ordinary model text. The Send button and keyboard use the same dispatch rule. `//` explicitly
escapes a literal leading slash; paths such as `/path/to/file` remain ordinary messages. Full native
command discovery/execution needs separate harness adapters and is not implied by this menu.

## Internal agent

`Agent.internal` is an optional boolean that defaults to `false`. The bootstrap migration creates
exactly one internal agent on `Service::open`, named "Monitter Admin"; that record is created only by
the bootstrap, is never offered for creation by ordinary `save_agent` callers, and cannot be deleted.
The existing admin record is selectable in Settings → Agents so its provider, model, host, folder,
and permissions can be changed. The internal agent and its single resident task are hidden from
conversation surfaces:

- Sidebars, the monitter menu, the Agent directory, and Cmd-K / Cmd-P palettes list user agents
  only; the internal record is omitted from those projections, search results, and pickers.
- New chat, channel invite, channel membership, and channel recipient pickers exclude the internal
  agent. `create_task` rejects it as a chat recipient; `set_channel_membership` is a no-op for it.
- Extensions MCP/skill eligible-agent sets, collaboration discovery, and the resident routing
  eligible-agent set all filter out the internal record before iteration.
- Archived chats, the share control, mobile projections, and paired-mobile / controller replies
  never surface the internal agent or its task. Visitor snapshots exclude them by construction.
- Run detail, the right sidebar, and any agent-attributed chat metadata skip the internal record.

Editing the existing admin via `save_agent` preserves `internal: true`, the canonical "Monitter
Admin" name, and `collaborationEnabled: false`. Generic callers cannot create a new internal record
or clear an existing one. When the admin's configured provider, model, host, cwd, or sandbox changes
through ordinary agent edits, the next `autoname` run safely replaces the resident task/session so
the new transport starts cleanly without inheriting the old native session id.

## Cross-agent collaboration

Agents publish `expertise`, `responsibilities`, and `skills` string arrays alongside their description.
`collaborationEnabled` defaults to true; disabling it removes the agent from harness discovery and
prevents new routed deliveries. Agent profile and instructions are captured for a newly created chat;
editing a profile changes directory discovery immediately and the instructions of future chats.
New chat context begins with `You are acting as <agent name>. You are an agent running
inside the Monitter harness.`, followed by `Your user is "<Profile name>".` when a
name is configured, then the saved purpose, expertise, responsibilities, skills and
freeform instructions. This saved system context is included in the first direct
or channel delivery (including a queued first turn), not repeated in each follow-up.
Changing Profile or agent settings does not rewrite existing chat context or history.
Profiles contain up to 40 entries per field, each at most 512 bytes. Full instructions and host
credentials are never included in the directory.

`Snapshot.collaborations` defaults to an empty array. Each record has an ID, kind (`message` or
`delegation`), originating agent/task, recipient agent/task, original text, caller-scoped request ID,
status, result/error and timestamps. `Message.senderAgentId` and `collaborationId` are optional and
identify real peer messages/results. These records and their task links are persisted atomically.
The Agent directory is available through the monitter menu and Cmd-P; profile editing and task run
detail expose capabilities, collaboration availability, linked chats, delivery state and results.

`Snapshot.subagentSessions` defaults to an empty array and is the durable, compact user-facing
projection for all delegated work. A session has stable `id`, `source` (`codex`, `acp`, or
`collaboration`), `parentTaskId`, optional native `parentThreadId`, `agentThreadId` and `agentPath`,
optional `collaborationId`, prompt/model/reasoning effort, status, result/error, and timestamps. Native
Codex items `collabAgentToolCall` (`spawnAgent`, `sendInput`, `wait`, `closeAgent`) contribute their
receiver thread IDs, prompt/model/effort and `agentsStates`; `subAgentActivity` contributes its path and
lifecycle (`started`, `interacted`, `completed`). An ACP agent that advertises the `subagents` client
capability (sent unconditionally in Monitter's `initialize` handshake; currently honored by
claude-agent-acp for its Task/Agent tool) may instead push `subagent_spawned` (identity, keyed by
`agentThreadId` = its ACP `subagentSessionId`) and `subagent_state_update` (`completed`, `failed`,
`disconnected`, `cancelled`) notifications on the parent session; an agent that does not advertise
support keeps folding that activity into the root transcript as before. Monitter delegations contribute
the same projection from their authoritative collaboration record. The latest projection is persisted in
SQLite, retained in compact LAN snapshots, and is therefore not lost when the event timeline is
limited, paged or restarted. Terminal native observations are sticky so late interaction notices cannot
show a completed sub-agent as running again. Raw provider activity stays available in the normal
diagnostic event store.

Unlike Codex, an ACP subagent has no separately re-queryable native thread: its own nested
`tool_call`/`tool_call_update`/`plan`/message/thought notifications arrive tagged with its
`subagentSessionId` instead of the root session, and are captured inline into
`Snapshot.subagentTranscripts` (keyed by the subagent session `id`, capped at 200 entries per
subagent) as they stream, rather than fetched on demand.

The composer subagent bar contains active sessions only. It is also the visor tab strip: opening a
tab lifts that same strip and reveals a live transcript panel rather than rendering a second set of
tabs. Monitter-routed delegations read transcript messages from their child task; native Codex
subagents poll the bounded read-only transcript command while their visor is open, which for `acp`
sessions instead returns the inline-captured entries directly. A terminal status removes the session
from the bar/visor immediately, while the complete durable list continues to populate the right
sidebar's separate Active and Recent sections.

A process-local broker binds only to `127.0.0.1` on a random port. Every running task receives its own
in-memory bearer grant; the broker derives caller identity from that grant, never from tool arguments.
Grants are revoked when the owning process is released. Resident Codex/Claude/ACP sessions retain
their configured grant across turns, but every tool call still requires the task to be running and
the agent's collaboration setting to be enabled. Replacement transports preserve the existing
ownership checks; restarting never replays an uncertain prompt.

The Rust broker exposes Streamable HTTP MCP at `/mcp`, using JSON responses and no SSE stream or
MCP session ID. Initialization supplies usage instructions; `tools/list` supplies the fixed schemas.
Bearer authentication applies to discovery as well as tool calls. Notifications receive an empty
202 response; GET is unsupported (405). The Python stdio relay and `/rpc` endpoint are removed.
The same Rust catalogue supplies the native harness tool allowlists:

- `skills_help`: explain supported shared skill installation and activation.
- `list_shared_skills`: list shared skill metadata without private MCP configuration.
- `install_shared_skill`: download public Markdown instructions for all current/future user agents.
- `list_agents`: search configured, published agents by profile.
- `delegate_task`: create a child chat, inherit its parent's project, apply the recipient's saved
  host/folder/model/permissions, and durably queue the supplied brief.
- `send_message`: send a brief to a peer, optionally using a directly connected collaboration chat.
- `get_task_result`, `wait_for_task`: read the actual result and incoming peer messages.
- `list_messages`: deliver queued messages to this active turn and return its durable inbox.
- `cancel_delegation`: cancel an active delegation owned by this task.
- `terminal_run`: open an interactive Monitter terminal tab and run a shell command in it.
  Requires `command` (bounded to 4096 bytes); optional `cwd` defaults to the caller task's
  saved folder. The shell stays interactive after the command finishes.

Repeated writes with the same caller task and request ID return the same delivery; changing its
recipient or message is rejected. A sender cannot address arbitrary existing chats or cancel another
agent's unrelated task. New task creation rejects self/ancestor cycles, more than four ancestor
levels, more than 24 requests per task or 64 per root chain. At most four collaboration runs dispatch
concurrently. Native session writer protections still apply; busy recipients remain queued.

Incoming peer messages become visible to an active recipient in `incoming_messages` during result
or inbox reads. This acknowledges delivery to that turn and prevents a second automatic delivery turn.
It does not claim that the recipient completed the sender's request. Otherwise an idle recipient runs
its queued message normally. Delegated results return to the source chat with the sender's identity;
an agent can wait for the result or inspect it later. Peer context is not new user authorization.

On restart, never-started queued deliveries remain queued. In-flight deliveries are marked interrupted
with explicit delivery uncertainty and are not automatically replayed. Cancelling a source chat
cancels its active collaboration descendants. Deleting chats with pending collaborations is rejected.
User follow-ups include recent peer results that arrived outside the native harness transcript.

Codex receives invocation-only `mcp_servers.monitter` overrides, required startup, a fixed tool
allowlist and approval scoped to these Monitter tools. Claude receives invocation-only MCP config
and an exact Monitter tool allowlist. OpenCode uses its inline config layer; existing native permission
rules remain in force. These are added to normal host configuration, without replacing authentication
or disabling other configured MCP servers. Hermes can receive tasks through its existing gateway
adapter; publishing callable collaboration tools into a Hermes turn awaits its ACP integration.

Local harnesses use the broker's loopback `/mcp` URL. SSH uses an owned reverse forward bound to
remote loopback; the remote harness receives that forwarded port, not the desktop-only port.
No MCP executable, Python relay, or temporary helper file is uploaded. Task credentials travel over
SSH stdin into the remote environment (or private ACP session frames); they never enter command
arguments, persisted state, command previews or logs. No public listener or persistent remote service
is installed. Codex uses `bearer_token_env_var`; Claude and OpenCode use native environment
interpolation for the Authorization header. ACP receives an HTTP server definition over its private
session pipe and must advertise `agentCapabilities.mcpCapabilities.http`; otherwise startup fails
visibly before a prompt is sent. Other Python-based adapters and SSH supervisors are unaffected.
The broker rejects non-loopback browser origins, invalid/revoked grants, oversized input and unknown tools.

## Task Git viewer

`get_task_git_status(taskId)` and `get_task_git_diff(taskId, path, scope)` inspect only the saved
folder and saved host of that task. They never accept a command, repository root or arbitrary
working directory from the UI. Both commands derive repository membership again for each request.
A non-repository folder returns `{ repository: false }`; it is not an error.

A repository status returns `{ repository: true, root, branch, files, truncated }`. `branch` is
null for detached HEAD. Each file has its repository-relative `path`, nullable `originalPath` for
a rename/copy, `indexStatus`, `worktreeStatus`, and `untracked`; a file can carry both index and
worktree changes. Status comes from NUL-delimited porcelain output and is bounded; `truncated`
means the returned list is incomplete.

A diff requires one exact path returned by current status and an explicit `staged`, `unstaged`, or
`untracked` scope. The response is `{ repository, path, scope, text, truncated, binary }`.
Untracked previews use a read-only no-index diff. Binary and oversized results are labelled rather
than expanded indefinitely. Git runs with optional locks disabled and external diff/textconv
disabled. Local and SSH execution use argument-safe invocation; the backend never stages, writes,
reverts, commits, fetches, changes authentication or runs a shell built from UI input.

## Archived permanent deletion

`preview_task_deletion(taskId)` returns `{ supported, reason, files }`. It is a read-only preview of
verified native session files. `delete_archived_task(taskId, removeNativeFiles)` permanently removes
Monitter's archived chat only after it repeats archived/not-running and pending-collaboration guards.
When native cleanup is requested, it repeats the preview checks before removing any file. Native cleanup
never removes a directory, auth, configuration, index, cache, symlink or non-regular file. It preserves
Monitter history if native cleanup cannot complete. Codex local and SSH JSONL sessions are eligible only when a
valid native UUID is uniquely owned by this host/provider/task and a bounded scan verifies matching
session metadata below canonical `sessions` or `archived_sessions`; other providers report a clear unsupported reason.


## Resume in Monitter

Resume and `/resume` start a continuation turn through the existing native CLI adapter with the
chat's saved native session ID, host, working folder and permissions. Output streams into the same
chat and Stop controls that owned run. The visible continuation asks the agent to continue where it
left off, or acknowledge completion and wait if the previous request is already complete. Unsent
composer text and attachments are preserved. Invalid, archived and already-running tasks are rejected
before transcript changes. Errors stay visible. Resume does not create another chat, copy a terminal
command, or take over an independently running Desktop/TUI process.

## Pane layouts and navigation

The main workspace offers one pane, two columns or a 2 × 2 grid. Each pane owns its tabs, selected
chat/channel, drafts and right sidebar. Dividers resize with pointer dragging or arrow keys. Tabs move
between panes; dropping toward a pane edge previews and creates a split, up to four panes. Collapsing
the layout merges tabs and preserves drafts and uploaded references. Layout changes wait while a send
acknowledgement or attachment upload is pending. Pane layout and unsent drafts are window-local.

### Agent and project workspaces

The desktop has an All workspace plus a workspace for each agent and project. Selecting an agent
or project changes the entire right-hand area, including every split pane. Each workspace remembers
its open tabs, selected tab, split layout, active pane and terminal references. Chats are opened on
demand; entering a workspace does not open all matching chats. Agent workspaces show that agent's
chats across projects; project workspaces show the project's chats across agents. Unassigned chats
have a No project workspace. Activity opens All; cross-agent channels open there as well.

These are views of shared task IDs, not copies of conversations or harness sessions. The same chat
can have a tab in its agent, project and All workspaces. Closing a chat tab changes only that view.
Switching workspace never sends a prompt, resumes/stops a harness or closes a terminal. Existing
chat composer text and attachments follow the chat across workspaces. New chat inherits the agent
or project scope; choosing a different owner routes the draft to a compatible workspace.

The selected workspace remains visible independently of the selected chat. There is no workspace picker:
selecting an agent or project changes context, while choosing Activity returns to All. The active agent
avatar sits in the desktop chat header, while the project icon remains in the tab bar; the overview
is not a draggable tab. Clicking the sidebar wordmark or compact mark returns to the independently
persisted All workspace and opens its whole-app overview without altering agent/project workspace state.
That overview occupies the workspace without a desktop tab bar or duplicate terminal shortcut; mobile
retains only its Back to chats navigation. Agent/project sidebar
metadata shows a compact local-disk or remote-cloud host indicator beside the harness/model line.
Badges and the global approval entry surface pending approvals even in hidden workspaces. Opening an
approval navigates to the shared owning chat. The left sidebar is always global: its Agents,
Projects, Activity and collapsed-rail chat lists show chats from every workspace. Selecting one routes
the right-hand area to its owning agent or project workspace before opening the chat. Global search can
cross workspaces; tab cycling and split/move operations act only inside the current workspace.

Agent/project sidebar subnavigation and collapsed-agent popovers list chats with open tabs first,
counting tabs in every pane and workspace. Other chats appear below a “Recents:” divider with a
subtle dimming; hover/focus restores full contrast. Existing ordering is preserved within each
group. Opening/closing a tab updates grouping without archiving or removing chat history.
At viewport widths of 760px or less, navigation uses two screens: the full sidebar list,
then the selected workspace. “Back to chats” returns to the list without closing tabs or
discarding drafts. This responsive state does not overwrite the saved desktop collapse preference.
Mobile screen changes slide horizontally; inactive screens are inert, and Reduce Motion disables
the animation. Screens stay mounted so draft and scroll state survive Back navigation.
Narrow workspace headers use the current tab title as a toggle for an open-tab picker only when
the pane contains more than one tab; a lone tab remains a regular tab. Selecting a tab closes the
picker. Desktop keeps its horizontal tab strip.
Touch screens do not use hover-to-reveal actions: secondary controls remain available,
avatar hover overlays are disabled, and mouse-style sidebar/tab dragging does not consume
touch gestures. Mouse hover affordances remain on fine-pointer devices.
Mobile shells follow VisualViewport height/offset as Safari browser controls and the keyboard
move, include safe-area padding, and use at least 16px editable text to avoid focus auto-zoom.

UI persistence uses `monitter.workspaces.v2` with an active workspace key and a collection of the
existing pane snapshots. Migration retains `monitter.workspace.v1` and imports its complete layout
into All. A failed or unsupported saved-workspace read must not overwrite the stored data. Hidden
workspace terminal references are preserved too; reload reuses live shells, while app restart can
open fresh shells at their saved host/folder, as described in Terminal tabs. Deleted/reassigned chats
are reconciled against current agent/project membership without deleting their history or drafts.

The main sidebar can collapse to agent avatars with chat popovers. Its client-local mini-tab row selects
Agents, Projects or Activity; non-Agent chat rows include the owning agent's avatar. Chat archive
buttons appear on hover or focus. The monitter menu contains Preferences, Hosts, Agent directory and
Archived chats. Cmd+, opens Preferences on macOS; Ctrl+, is available on Windows/Linux, alongside the
platform equivalents for the other shortcuts. The right sidebar uses a dismissible blade when its chat
pane is narrow. Menus use the browser top layer so pane overflow cannot crop them. Jump-to-latest is
centered immediately above the composer. Send is an icon, startup uses a spinner, and current-turn
activity shows Stop; both startup and stepping can be cancelled once a task exists.

## Attachments

`store_attachment { target, filename, mimeType, dataBase64, previewDataUrl?, sourceId? }` stores a
regular file up to 20 MiB under the destination cwd's `.monitter/attachments`. `target` is either a
saved `taskId` or a new draft's `agentId` plus optional `projectId`; the backend derives host/folder.
The returned Attachment includes id, name, mimeType, size, path, previewDataUrl and optional sourceId.
`read_attachment_file { sourcePath }` reads a native file drop with the same size limit. Browser paste
and file picker supply bytes directly. New attachment directories are private 0700 and files 0600;
symlinked storage directories are rejected before creating children. SSH writes use bounded transport
and native host configuration. Original files are never moved or deleted.

A private persisted registry binds attachment IDs to their exact host and folder. Sends accept only
registered IDs for that destination and append JSON-quoted paths to the harness prompt while keeping
ordinary message text unchanged. Attachment-only messages are valid. New drafts upload without creating
a task or starting a harness. Channels store one copy per distinct recipient host/folder, route only the
matching references to each recipient, and group copies by sourceId for display. Changing a draft's
folder or channel recipients clears mismatched queued references with a visible explanation. Removing
a queued attachment does not delete stored files. Files remain under `.monitter` after chat deletion.

Persisted chat/channel messages show a file type icon or bounded embedded image thumbnail. Queue state
and unsent text survive navigation and tab movement within the window. Failed uploads/sends remain
visible and preserve successfully queued files. Thumbnails are at most 192 pixels and 256 KiB; missing
image previews fall back to a file icon.

Opening an uploaded image uses `read_attachment_image { attachmentId }` and returns
`{ filename, mimeType, dataBase64 }` from the registered original without resizing or re-encoding.
The reader is available to native and authenticated LAN clients, not visitors. It accepts no
caller-supplied path, enforces the saved attachment folder and 20 MiB limit, and allows only
PNG/JPEG/WebP/GIF raster images. Missing or unsafe originals produce a visible error rather than
an enlarged thumbnail. Original bytes load on demand, not with transcript history.
Codex receives original attachment paths in its text prompt, not inline image input or thumbnails;
the agent must open the file. This does not guarantee provider-internal full-resolution processing.

Codex Computer Use MCP result images are distinct from user-uploaded `local_image` records.
Supported PNG/JPEG/WebP result bytes can be retained inline on the following assistant reply,
bounded to four pending images and 512 KiB per encoded data URL. Invalid or oversized images
produce a visible error; finishing a run clears any unassociated pending images. No arbitrary
host filesystem reader is exposed to LAN clients. Persisted image attachments display at chat
width, while removable composer attachments remain compact previews.

## Composer models and pane appearance

The composer shows its current model beside Send, with the attachment button at bottom left.
Codex catalogs come from that task's saved host (local or SSH) through read-only app-server
`config/read` and paginated `model/list`, cached by host/provider/working folder for 60 seconds. The menu exposes
only advertised models, reasoning levels and Fast capability. Other harnesses currently report that
catalog discovery is unavailable; they keep their configured models. Catalog failures remain visible.

`ModelSettings` contains `model`, nullable `reasoningEffort`, and nullable `fastMode`. New drafts keep
these choices locally until their first send. Existing idle chats retain native session ID, host,
folder and history when changing model. Running Codex chats accept changes that take effect on
the next `turn/start`; running chats from other harnesses (Claude, OpenCode, Hermes, ACP) still
reject because those harnesses carry model/sandbox flags at process launch. Archived chats always
reject. Codex receives
invocation-only model/effort/service-tier overrides; Fast uses the advertised `priority` tier, explicit
off uses `default` only for models advertising that capability, and resetting clears overrides.
Models without advertised Fast support send no service-tier override. User CLI configuration and authentication are
never edited. The Resume icon appears on interrupted/error native sessions; the backend also rejects
Resume while the previous owned process is still active. Ordinary completed chats continue on send.

Every pane uses the same tab-bar height, including native macOS zoom compensation. Split separators
paint a one-pixel line with a wider invisible drag target. Agent messages show the sending agent's
small avatar. Closing or successfully moving a pane's last tab removes that pane once in-flight
actions finish; cancelled transfers preserve it and terminal transfers keep their live sessions.
The sole remaining pane stays as the workspace's empty view, and newly created empty splits are
not removed without a close or completed move.
`Settings.dimInactivePanes` defaults to true and `inactivePaneOpacity` to 0.6; Preferences
provides a toggle and 10–90% slider, with validation requiring a finite value from 0.1 to 0.9.
Pointer or keyboard focus immediately marks the receiving pane active; its opacity is always 1.


Dimmed panes also desaturate completely; focusing a pane or disabling dimming restores its colour.
Task and channel title backgrounds are fully opaque with no backdrop blur. Both use 8px vertical
and 15px horizontal padding, a compact title, and an avatar or group icon in the same left-hand slot.
Headers sit above the message scroller and span the chat and its right sidebar. Mobile uses its
compact tab selector for multi-tab panes without adding a duplicate pane title header.

Consecutive tool entries of the same tool or connector family share one muted, borderless row
with extra spacing below. Connector methods such as `gmail.search_emails` and `gmail.read_email`
share a family; the visible description and timestamp follow the latest entry. Clicking opens a
bounded, scrollable top-layer popup containing the full list and each entry's original details.
Escape or clicking outside dismisses it. Matching streamed entries extend an open popup.
Grouping is presentation-only: raw events, details, timestamps and settings remain unchanged.
Messages, reasoning and different services separate groups. Counts say “entries” because harnesses
can emit multiple lifecycle updates for a single call. Individual entry details can collapse.

ACP tool activity retains the provider session and tool-call identity. The transcript merges
start/progress/completion updates for a call into one entry, renders its reported kind and
status, and keeps failed calls visible separately from successful calls. Plan updates replace
the prior plan within the same user-message boundary. Full tool details remain in the diagnostic
journal. Anonymous assistant text starts a new message segment after tool activity so commentary
and the final reply are not concatenated across tool calls.

## Terminal tabs

The terminal icon in each pane (also Controls → New terminal) opens a real interactive shell.
The current chat supplies its saved host/folder; drafts and agent/project views use their configured
folder, including per-host project folders. With no context it uses the local host's default folder.
Local shells are login shells in a PTY. SSH shells use the saved address, user, port and identity,
with strict existing host-key checking; a failed remote folder change cannot silently open elsewhere.
Terminal tabs support the existing pane drag/split/merge behavior and appear in the switcher.
Switching tabs or panes retains the same shell and screen. Closing a terminal ends its owned session.
Workspace restoration reuses a still-live terminal after a frontend reload; after an application quit
it opens a fresh shell at the saved host and launch folder. The old shell process, screen, current
directory changes and session ID do not survive a quit.

IPC: `open_terminal {target,cols,rows}`, `list_terminals {}`, `write_terminal {id,data}`, `resize_terminal {id,cols,rows}`,
`read_terminal {id,afterSeq}`, `close_terminal {id}`. Target resolves saved task, agent/project, host,
or an explicit validated `cwd` on its host
configuration; returned TerminalSession contains id/title/hostId/cwd/status/exitCode. Reads contain
sequenced byte chunks, `nextSeq` (last delivered sequence), status/exitCode and a truncation flag for
actual ring loss. Native buffers retain at most 1 MiB; each read is at most 256 KiB. Terminal input is
bounded to 64 KiB per write, sizes to 10–500 columns and 4–300 rows. xterm retains 10,000 scrollback
lines; UTF-8 decoding spans chunks. Reads/input are serialized and late mounts cannot steal a terminal
from its current pane. Close failures stay visible and can be retried.

Terminal titles start as `Terminal`. For local native PTYs, the backend reads only that PTY's
foreground process group (never a process-wide job list): after the same foreground shell/process
has remained for five seconds, an idle shell becomes `Terminal: <actual-shell>` and a foreground
command becomes `Terminal: <process>`. Background jobs do not affect this title. Unknown process
state keeps the existing truthful title rather than inventing an idle/command state. The monitor is
backend-owned, so visible, hidden and sidebar terminal consumers receive the same state through
ordinary `read_terminal`/`list_terminals` projections. `TerminalSession.title` is the display title;
it also returns `autoTitle` and `customTitle`. An explicit terminal Auto-name is stored as
`customTitle`, takes display precedence for that session, and is never overwritten by automatic
process tracking; automatic tracking continues in `autoTitle`. Each terminal read includes its
current TerminalSession projection.
Process-group leaders can disappear before another member of a pipeline; when the native single-PID
lookup cannot resolve a reliable leader, the existing title is retained rather than guessing.

Shell Ctrl-C/Ctrl-P and other unshifted Ctrl combinations remain terminal input, except Ctrl-W on
Windows/Linux, which closes the active Monitter tab. On macOS, Cmd-W closes the active Monitter tab;
Cmd-K/P,
Cmd-comma and scale shortcuts continue to control Monitter. Ctrl-Shift-K/P accesses Monitter palettes
while a terminal is focused on other platforms. Terminal rendering uses
[xterm.js](https://xtermjs.org/docs/api/terminal/classes/terminal/) and native PTYs use
[portable-pty](https://docs.rs/portable-pty/latest/portable_pty/).

## Run detail and timeline

The right sidebar separates Run detail, Timeline, and (when available) Git changes. Run detail keeps
agent identity and settings, adds a compact branch/staged/changed/untracked summary from the existing
Git status poll, and lists running Monitter tasks and terminals in the same host/folder. This list is
labelled Tracked processes: it does not claim to enumerate arbitrary OS processes or undisclosed jobs
inside a harness. Timeline owns diagnostic events, with bounded previews and expandable full output.
Each tab scrolls within the sidebar; panels preserve their selected detail tab during pane movement.

`Settings.focusFollowsMouse` defaults to false and is available in Preferences and Controls.
When enabled, entering a pane with the mouse activates it and directs keyboard focus to its current
terminal, composer, or message scroller without scrolling the view. Touch, drag selections, resizing,
and open menus/dialogs do not trigger hover focus. Disabling it restores click/keyboard activation.

## Settings workspace tab

Preferences, Cmd/Ctrl-comma, Cmd-P Settings, and the Cmd-K Settings entry open or focus a single
Settings tab in the workspace. It closes with its close icon or Cmd/Ctrl-W, moves between panes
(including edge-created splits), and restores its location and selected category with the workspace.
Its category navigation and content scroll inside the pane; it does not block other tabs.
Categories include Profile, Agents, Agent directory, Appearance, Typography, Permissions & Behaviour, Conversation, LAN access, and desktop Remote control. Existing controls
save automatically. Permissions remain per-agent/task; the page does not introduce a global bypass.
Settings edits and command-palette preference changes use a shared queue that merges each patch
into the latest saved settings. Failed saves remain visible; switching tabs preserves the settings
page's local state and the chat's unsent draft.

Profile contains an autosaved Your name field (`Settings.userName`). It defaults to
an empty string for new and older stores: Monitter never guesses a name. Names may
contain at most 80 Unicode code points and no control characters; the UI trims
surrounding whitespace and clearing the field omits the user-name sentence in new
chat prompts. This owner preference is shared by desktop and LAN settings. It is
display context only, not authentication, permission or shared-operator authority.

Conversation display: `Settings.tintUserMessages` defaults to false for existing and new installs. When enabled, user bubbles in direct chats and channels use a subtle accent tint; message content and agent replies are unchanged.


`autoname { target: { taskId?: string, channelId?: string, terminalId?: string, content?: string } }` returns Snapshot. Exactly one target is required; terminal text comes from a bounded, control-sequence-scrubbed client buffer. Chat/channel context uses the last 12 messages, bounded to 12,000 characters. The naming request is a blocking resident turn on the resident Monitter Admin agent: `Service::send_admin_turn(prompt)` → `AdminTurnBroker` → the saved Codex app-server / Claude stream-json / ACP transport selected by the admin's configured provider. The admin's configured provider, model, host, cwd, and sandbox are used exactly as saved — there is no Spark/Luna/Mini mini-model selection, no provider fallback, and no automatic retry. The request is bounded to 45 seconds; on expiry the resident control is terminated and the next explicit turn starts a fresh resident transport. At most one admin request is active at a time; concurrent callers are rejected with `Monitter Admin is busy with another interface request.` Internal prompts, replies, and request IDs are runtime-only and never enter `Snapshot.messages`, `RunEvent`s, `get_task_events` pages, compact LAN snapshots, or shared/visitor projections. The saved native session id on the internal task is persisted so a restart resumes the saved native session; an interrupted, timed-out, or failed request is never replayed. Titles are bounded to 60 characters. Terminal names update the existing session and are refreshed into the tab runtime. The final target mutation is reported through `monitter:changed` exactly as for ordinary renames. Use /autoname in chat or find /autoname in the Controls palette (Cmd-P on macOS) for a terminal; the shell never receives the command.


### Pane controls and window chrome

The main sidebar toggle is the first control in the main pane tab bar. Its centered responsive
mark/wordmark sits above the Agents/Projects/Activity mini-tabs, with New terminal and New chat
actions at the right of the brand row. Native macOS window-control clearance is
removed in fullscreen and restored on exit, using the native window fullscreen state.
Right-sidebar toggles live beside expansion in chat/channel headers only, because those are
the pane types with run detail or channel-member sidebars. Compact sidebar blades retain
their own close control. Expansion controls remain reachable when their tab bar is hidden.


`Settings.compressToolCalls` defaults to false. When enabled, calls from the same tool
family are aggregated across intervening thinking, reasoning summaries, and context
compaction within one message-bounded turn (for example `Searched the web · 6 tool calls · 4.6s`).
Messages and resolved approvals remain boundaries. The duration is the recorded
first-to-last event span, not summed tool execution time. Clicking expands an inline
scrollable box containing every original event; disabling compression restores adjacent
family grouping. Empty thinking statuses are transient and disappear after later visible
activity arrives. Stored events remain unchanged.

Queue edits update only the selected unsent delivery. In channels, each recipient's queued
delivery is independent; already posted channel history or deliveries to other agents are
not rewritten. Empty text requires a retained attachment. Claiming a queue item and editing
it use the same state lock, so an edit cannot silently replace an already dispatched prompt.


## Agent conversations in channels

Channel configuration includes `agentConversationEnabled` (default false),
`agentConversationTurnLimit` (default 6, range 1–20), and runtime
`agentConversationTurnsUsed`/`agentConversationPaused`. Dedicated commands
`set_channel_agent_conversation {channelId, enabled, turnLimit}` and
`stop_channel_agent_conversation {channelId}` manage this feature. Ordinary channel
edits preserve the runtime counters. The member sidebar exposes these controls.

Only completed agent replies can request another member through an explicit mention.
Automatic deliveries share a per-channel budget, remain in that channel, and queue when
the recipient is busy. Peer requests retain their sender identity and are context rather
than new user authorization. A user-written channel message begins a new round.
Stopping pauses routing before cancelling channel work; disabling prevents further
automatic deliveries. Enabling does not replay old channel history.

Queued peer requests use `origin: "channel-agent-mention"` and `senderAgentId`. They
show their originating agent and recipient in the queue and can be removed, but not
rewritten as if the agent authored new text. User-authored queued messages remain editable.

## LAN browser access

The desktop hot-reload interface is separate from LAN browser access. An installed app may load
`http://127.0.0.1:18420/monitter-app-ui/?monitter-dev-ui=1` only after a native compatibility-marker
probe succeeds. The marker is not an authentication mechanism. Its Tauri capability is restricted to returning to the packaged main-window URL; arbitrary
hosts, LAN peers, the `dev:web` port, and the developer renderer receive no native task, terminal,
filesystem, approval or settings IPC. The renderer instead uses the existing localhost developer bridge,
which uses the normal local API access policy. While `REQUIRE_ACCESS_CODE` is false, hot UI is
automatically trusted and needs no pairing code. The loopback-only Vite proxy rejects originless
and foreign write requests before rewriting Host/Origin, and overwrites the legacy developer-bridge
marker with `0` so it does not enable that separate code gate. Re-enabling the global API code
requirement also protects this proxy. The frontend displays a persistent `DEV UI · HMR`
badge and returns to packaged assets after three consecutive marker failures. This mode reuses the
installed app's native process rather than starting a competing Tauri backend.

For UI iteration, `npm run dev:web` starts Vite in `monitter-web` mode on port 18450. It uses the
running desktop's API on loopback port 18436 with the existing access-code and approval checks.
The dev server listens on IPv4 and allows only loopback, private-LAN and Tailscale peers using a
current local interface IP. Its proxy rejects foreign origins and originless API writes before
rewriting Host/Origin for the trusted loopback hop. This is an owner interface, not a read-only
preview. Use the IP URLs printed by Vite; restart it after local IP/interface changes. `--port`
can select another free port. It does not publish LAN assets or replace the installed app.
Svelte/CSS changes use Vite HMR; backend changes still need a desktop rebuild. Normal `npm run dev`
and production builds do not enable the dev proxy or dev-only browser bridge.

The desktop app owns an internal HTTP server on port 18436. It prefers live web assets
in the app data folder's `lan-web` directory, with bundled assets as a fallback.
On macOS, `npm run build` builds and publishes there without rebuilding Rust or restarting
Monitter. `npm run publish:lan` publishes an already completed build. Publication installs
assets before atomically replacing `index.html`, retaining old hashed chunks for open tabs.
Refresh the browser to load the new interface. This updates LAN browsers only; the desktop
webview remains bundled. Backend/API changes still require a matching app rebuild.
Settings → LAN access displays local addresses and a random
per-process six-digit access code (`get_lan_server_info {}` -> `{urls, token, error, accessCodeRequired}`;
the `token` string preserves leading zeros). Five incorrect guesses lock API access
for five minutes across all clients; successful polling does not reset the counter. The server
starts with the app and stops with it. A bind failure is shown in Settings.

During LAN-only prototyping, `REQUIRE_ACCESS_CODE` is false: the code gate and guess
limiter are bypassed, while private/loopback peer, literal local Host and same-origin checks
remain enforced. `GET /api/access` returns `{required:false}` so browsers skip the gate.
Anyone on the trusted LAN has owner-level access, including terminals. The code and limiter
implementation remain available to re-enable later; this is intentionally not Internet-facing.

Browser API calls use `POST /api/invoke {command, args}` with a Bearer access code;
responses are `{ok: true, result}` or `{ok: false, error}`. The browser sign-in gate
stores the code in sessionStorage. Access links place the code in a fragment, which is
removed after loading. Static assets are public; workspace data and commands require
authentication and same-origin requests. The code grants owner-level workspace control,
including terminals; this is distinct from limited visitor sharing. HTTP is unencrypted
and intended for a trusted LAN. Restarting the app invalidates browser access codes.

The full interface uses the same compact revision-based UI protocol as the desktop. Browser
refreshes are coalesced so slow requests cannot overlap; unchanged revisions do not replace
the UI state. Send acknowledgements do not wait for a subsequent snapshot fetch. Native filesystem
pickers, arbitrary native file reads, and quitting the host app are not browser actions;
uploads use browser file bytes. Work executes on the host Mac or its configured SSH hosts.

## Visitor chat sharing

Visitor sharing is a temporary capability separate from owner-level LAN access and
remembered mobile controllers. A share is bound to one approved remote public key,
one named visitor, and an explicit set of chat IDs. The invitation is a one-use
rendezvous: presenting it does not grant access until the desktop operator compares
the verification code and approves the named visitor. A second peer cannot join the
same active invitation. Ending the share, rejecting the visitor, closing the desktop
session, or removing a chat from the selection revokes that access.

The share URL uses an explicitly configured, publicly reachable HTTPS origin serving
the visitor `/share` application. A Tauri, loopback, or LAN owner-control origin is
never copied into a visitor link, and an owner access code is never embedded in it. If
no hosted visitor origin is configured, link creation fails visibly.

Each visitor request is authorized at execution time, not merely when received. The
desktop re-reads the authoritative snapshot and rechecks the live share generation,
approved peer identity, and exact selected chat immediately before invoking a command.
Revocation during an asynchronous check therefore denies the command. The visitor
command surface contains only the scoped snapshot read, bounded file upload, and
sending a message to an allowed, unarchived, non-channel chat. A visitor may upload
at most eight files per message, each at most 8 MiB, using encrypted 32 KiB chunks.
The visitor may include one optional client-generated PNG, JPEG or WebP thumbnail
per upload; it is a validated data URL capped at 48,000 characters, never a file
path or remotely fetched URL.
Pending encrypted chunks are peer- and task-scoped, expire when incomplete, and have
a bounded aggregate memory budget. Before the desktop stores an upload, and again
before it sends a message, it rechecks the immutable live share and exact selected
task; it accepts only attachment IDs created by that visitor for that task. Attachment-only
messages are valid. It does not expose task cancellation/resume, channels, terminals,
approvals, settings changes, filesystem or Git operations, attachment reads, agent
setup, or any other owner command.

Visitor snapshots are constructed as an explicit allowlist. They contain only the
selected tasks, their ordinary shared transcript, and the minimum agent/project display
records needed to render those chats. They omit hosts and local paths, native session
IDs, launch commands and arguments, saved instructions, attachment source IDs and
paths, system profile messages, activity and diagnostic events, collaborations, queued messages,
approval requests and remembered approval rules. Adding a field to `Snapshot` does not
make it visitor-visible automatically.

Human attribution comes from the desktop's approved operator identities. The visitor
cannot choose a message prefix or payload that causes the owner, an agent, or another
person to be shown as its author. The model prompt and both UIs distinguish the primary
user from the named visitor. Names are display identity rather than authentication
claims; the approved transport key is the authority for the active visitor session.

## Mobile controller

`docs/MOBILE-CONTROLLER.md` describes the paired mobile clients and relay.
Remote control is opt-in. The versioned controller allowlist dispatches only
approved encrypted sessions through the existing desktop bridge. Desktop and
mobile navigation stay independent. Initial pairing uses QR invitations or one-use
nine-digit rendezvous codes and explicit desktop approval after comparing
verification numbers. Owner-approved phones and desktop identities persist as
non-extractable P-256 CryptoKeys in IndexedDB. Each reconnection derives fresh
transport keys and proves the remembered phone identity before approval.
Remote control lists saved phones and permits individual or complete revocation.
Its inactivity policy defaults to indefinite; optional whole days slide forward
on authenticated access. Expired/revoked identities cannot renew themselves or
dispatch requests. Closing a session blocks further requests; already accepted
work continues. Phone foreground/resume and interrupted connections reconnect
automatically. Explicit Disconnect forgets the phone's saved pairing. Mutation
receipts remain session-local, so uncertain sends are never automatically retried.
Older desktops remain compatible but cannot remember approval until upgraded.

## Shared operators

Workspace sharing is a separate, opt-in capability built on the same ephemeral,
encrypted one-use pairing transport. The desktop owner enters their display name,
shares a one-use link or quoted nine-digit code, compares the verification code,
and explicitly approves the visitor's declared display name. Pairing by itself
grants no workspace access.

After approval, the owner selects individual chats and/or projects. The visitor
can view and send messages only within that current selection; they cannot stop
or resume agents, create chats, access terminals, inspect hosts, folders,
agent instructions, activity, approval details, queues or other workspace records.
They can upload only through the bounded per-share chat upload flow and can see
shared attachment filename, MIME type, size and a bounded raster preview; local
paths, source IDs and attachment-reading APIs remain private.
Visitor snapshots use an explicit field projection; new desktop snapshot fields
must be reviewed before they are exposed to visitors. Harness approvals remain
with the desktop owner, even for a shared chat.
Visitor projections omit the owner's Profile name and saved system instructions;
peer-attributed system messages remain visible as shared conversation content.
Ending sharing revokes the in-memory session immediately. A reconnect needs a
fresh link and approval.

For each selected task, the sharing projection may also include owner-computed
composer display metadata: the resolved model label/ID, reasoning effort, Fast
mode, and the same per-run context usage state shown by the desktop composer.
The usage lookup is `cache-only`; it projects only the selected task's matching
context `{ used, size }` when the harness reported one, otherwise an explicit
unavailable state. It never exposes usage-overview account allowances, other
runs, events, native session IDs, paths, catalog source/warnings, or lookup
errors. This metadata is read-only and does not grant model, sandbox, approval,
or other owner controls. Older owner bridges retain an explicit unavailable
context state and harness-default model display.

Shared user messages are stored and sent to the model with visible attribution:
`@(Alex): message`. The execution prompt also names the two operators, such as
`Alex (primary user)` and `Luke (visitor)`, so attribution is model context
rather than an implicit authority change. The UI renders the operator name and
avatar separately from the message text. Shared transcript attachments expose only a
redacted `path: ''`, filename, MIME type, byte size and a bounded safe image preview;
they never expose native locations, source IDs, attachment readers, terminals or
filesystem capability. The shared chat snapshot includes the approved primary and
visitor display identities, the owner's safe appearance projection, and the upload
limits so the visitor can render the ordinary read-only chat surface without reading
owner settings.
