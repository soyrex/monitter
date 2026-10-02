# Monitter desktop contract

Tauri 2 / Svelte 5 / TypeScript / Rust. macOS Apple silicon first.
Canonical TS data shapes: `src/lib/types.ts`. Rust serde fields use camelCase.
All timestamps are Unix milliseconds. IDs are UUID strings. Optional task links use null.
No fake conversations, progress, token counts, host connections or model replies in the shipped app.

## Commands (Tauri invoke names and JSON argument keys)

- Native browser commands are owner-desktop-only; they are absent from LAN, mobile, and shared
  visitor dispatch. All coordinates below are finite logical window pixels and `tabId` is a UUID.
  Browser views use an app-private browsing profile shared across browser tabs, never the Monitter
  shell's profile. External pages receive no Monitter Tauri IPC capability. Only inert `about:blank`,
  `http:`, and `https:` navigation is accepted. HTTP(S) URLs containing userinfo credentials reject;
  explicit Basic Auth credentials, extension state, and page contents are never
  written to workspace snapshots or command logs. Tab URLs and titles are saved
  as restore hints; sites can place secrets in URLs, so this metadata is private
  workspace data and must not be exposed to shared or LAN clients.
- `browser_open { tabId: string, url: string, bounds: { x, y, width, height } }` ->
  `{ tabId, url, title, canGoBack, canGoForward }`. History availability fields may be `null`
  when the native engine cannot report them. Creates or restores one native child webview.
  On macOS, profile-isolated browser tabs require macOS 14 or newer; older systems receive an
  explicit unsupported error rather than silently sharing the shell's profile. Browser-extension
  hosting, if enabled, requires macOS 15.4 or newer and must be validated per extension. Basic
  Auth is not advertised as supported: Monitter supplies no credentials/custom challenge callback,
  and WKWebView may reject an unhandled HTTP authentication challenge.
- `browser_set_layout { tabId: string, bounds: { x, y, width, height }, visible: boolean }` ->
  `BrowserState`.
  The renderer coalesces geometry measurements after layout, follows pane motion
  through its final frame, and suppresses unchanged native layout writes. Position
  and size are applied as one native rectangle. Layout revisions do not hide a
  still-visible child; teardown and visibility changes explicitly hide it.
  Pane/sidebar resize starts hide every native browser child before drag layout
  updates. A shell-rendered URL/browser placeholder fills its viewport while
  bounds continue updating invisibly. Pointer completion/cancellation restores
  visibility after settled frames; window-resize bursts use a short idle delay.
  No reload, navigation or native browser recreation is required.
  Address editing retains an unsubmitted draft across native state polling;
  Enter submits it and Escape restores the current page address. Native state
  refreshes are coalesced and discarded after tab retirement.
  Hiding a tab retains its live page; closing its view unloads it. Reopening an unloaded tab
  reloads its URL and may lose unsaved page state.
- `browser_navigate { tabId: string, url: string }` -> `BrowserState`; `browser_back { tabId: string }` -> `BrowserState`;
  `browser_forward { tabId: string }` -> `BrowserState`; `browser_reload { tabId: string }` -> `BrowserState`;
  `browser_close { tabId: string }` -> `()`; `browser_get_state { tabId: string }` ->
  `{ tabId, url, title, canGoBack, canGoForward }`. Native browser process and page state remain
  authoritative; local tab metadata is only a restore hint. Close is idempotent for an absent tab,
  hides the native child before teardown, and removes the local tab controls only after native
  cleanup accepts the close; a failed close retains those controls for retry. Workspace switching
  also waits for native browser retirement and stays put if any child fails to close. macOS reads and moves the native
  `WKWebView` history stack through a main-thread bridge. Windows and Linux use the corresponding
  WebView2/WebKitGTK native history APIs. Those portable paths compile only on their target
  platforms and still require native runtime validation before being advertised as supported.
- `monitter:browser-state` emits `BrowserState` when native navigation or document title changes.
  The owner desktop updates only the matching local tab; unmounted browser panes unsubscribe.
  The event never carries page content, credentials, or extension data.
- `monitter:browser-focus` emits `{ tabId: string }` to the local main webview when a mouse
  click lands inside a macOS native browser child. The root workspace activates that tab's
  owning pane without stealing focus from the website. The child page receives no Monitter
  command capability or injected script for this signal.
- `browser_load_unpacked_extension { path: string }` -> `{ path, displayName }` is an
  owner-desktop-only macOS 15.4+ operation. `path` must be an absolute unpacked
  directory with a bounded JSON manifest. A successful return means WebKit
  accepted and activated the extension context, not that its popup, permissions,
  native messaging, or autofill work. Direct Web Store installation is outside v1.
- `load_dev_ui {}` -> `()`. Owner desktop only. The native app connects directly to exact IPv4
  loopback port 18420, requires a compatible Monitter marker, and then navigates the existing main
  WebView to its dedicated Vite bridge path. It does not start a server, launch another backend, or
  accept an arbitrary URL. The remote renderer has no general Tauri IPC.
- `use_packaged_ui {}` -> `()`. Restores the packaged URL captured when the native app launched.
  It is the only custom Tauri command allowed from the dedicated developer bridge origin. Switching
  either direction reloads ephemeral frontend state, while the same native service, stored data,
  active harnesses and task sessions continue running.
- `get_command_capabilities {}` -> `{ protocolVersion: number, commands: string[] }`.
  Native and owner-LAN clients check the version and advertised command before invoking an operation.
  The native list reflects the checked manifest; the LAN list includes only its explicit LAN surface.
  Unknown handshakes on older backends allow only the manifest's legacy command baseline. A present
  incompatible version, malformed reply, authentication failure or transport error fails visibly.
  Mutations are never retried after ambiguous delivery. This compatibility check does not grant
  permission: Tauri ACLs and native resource/actor checks still apply. Controller and visitor action
  mappings remain separate restricted protocols. `use_packaged_ui` retains its narrow origin-specific
  bootstrap exception. See `docs/COMMAND-CONTRACT.md` for generation and validation.
- `owner_relay_invoke { command: string, args: JSON }` -> JSON. Bundled desktop renderer only;
  it dispatches through the existing explicit owner-LAN command allow-list, not the broader
  native Tauri command set. The paired iOS client can reach it only through a separately
  remembered, explicitly desktop-approved owner-workspace E2E relay grant. Controller-only
  trust never implies this grant; visitor/operator sessions cannot request it. Revocation and
  inactivity policy are rechecked for each relayed request. The desktop advertises
  `owner-workspace-v1` in its encrypted approval; an older desktop without that capability
  leaves the phone in limited paired mode with a visible update-required message rather
  than sending an unknown owner request or reconnecting in a loop. Encrypted requests and responses
  use ordered 512 KiB chunks when both peers advertise `owner-chunks-512k-v1`,
  otherwise ordered 32 KiB chunks. Both paths retain a 32 MiB aggregate ceiling,
  2 MiB encrypted-frame ceiling, 60-second transfer timeout, and client pacing
  below the public relay's 120-frames-per-10-seconds limit; ambiguous mutations
  are not replayed. After a completed voice request transfer, the paired iPhone
  waits up to six minutes for desktop Whisper transcription; other owner commands
  retain the one-minute response limit. The owner relay also carries two
  small, validated presentation-only operations in the desktop renderer for app palette,
  surface tint and terminal palette, which are client-local rather than Snapshot fields.
- `get_snapshot {}` -> Snapshot
- `get_ui_snapshot { revision?: string }` -> `{ revision: string, snapshot: Snapshot | null }`
  Shared desktop/LAN UI projection. An unchanged launch-scoped revision returns null; changed
  snapshots contain at most 300 recent activity events (60 per task), omit output/log payloads,
  and bound details to 1,000 bytes (300 for error summaries) rather than full diagnostic history.
  Task/channel messages remain complete for legacy consumers. The owner-side controller v1
  adapter uses a bounded compatibility projection for older clients: it retains task/agent identities, recent chronological conversation tails,
  and a visible per-task desktop-history notice while measuring the serialized response below
  its 1 MiB ceiling. Newer paired clients may request older task messages through the optional
  `task-message-pages-v1` capability and `getTaskMessages { taskId, beforeId?, limit? }` action.
  The cursor is a message UUID, the limit is 1-40, and the response is `TaskMessagesPage`
  (`messages`, `nextBeforeId`, `revision`). The action remains within the controller response
  ceiling and never bypasses visitor projection or revocation checks.
- `get_ui_delta { revision?: string }` -> `UiDeltaResponse`.
  Native and owner-LAN only. Initial reads or expired, foreign, oversized or unrepresentable journal
  revisions reset with a compact snapshot containing the latest 64 messages per task and channel. Incremental
  reads contain changed messages, explicit message removals, and replacement metadata. Unchanged
  channel/subagent histories are retained only by explicit IDs. A delta applies only to its exact
  `fromRevision`; otherwise the client fetches a reset. The launch-scoped journal retains at most
  128 revisions and 8 MiB of encoded changed task/channel messages. Changed channels include
  `channelMessageChanges` with message upserts/removals and an explicit `reset` for structural
  replacement; channel metadata carries empty message arrays in deltas. Full durable history remains available locally.
- `get_task_messages { taskId: string, beforeId?: string, limit?: number }` ->
  `{ messages: Message[], nextBeforeId: string | null, revision: string }`.
  Native and owner-LAN only. Pages are chronological, default to 64 records, and cap at 100. The
  stable message ID is an exclusive cursor; unknown/deleted cursors or cursors from another task
  fail visibly. Clients only merge a page at its matching snapshot revision. Loading earlier
  messages preserves the detached reader's anchor and frozen live transcript; returning to the
  bottom, sending or switching conversations releases queued live updates.
- `get_channel_messages { channelId: string, beforeId?: string, limit?: number }` ->
  `{ channelId: string, messages: ChannelMessage[], nextBeforeId: string | null, revision: string }`.
  Native and owner-LAN only. It uses the same chronological default-64/max-100 exclusive-ID page
  contract as task history. The cursor must exist within this channel. Clients validate channel ID
  and revision before merging; structural channel resets discard stale loaded pages.
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
  and resident memory for breakdown views. An owned CLI/ACP harness root also includes the
  optional `chatTitle` of its running task; processes without a known task owner omit that field.
  Unsupported platforms fail visibly instead of returning
  fabricated values. In the resource modal, a collapsed process group shows the sum of its own and
  all live descendant CPU/RAM usage; an expanded group shows its own usage beside the child rows.
  RAM value text is orange above 250 MiB and red above 500 MiB; row backgrounds are unchanged.
- `list_system_fonts {}` -> `{ family: string, monospace: boolean }[]`.
  Native desktop only. It returns local OS font family names for Typography suggestions, without
  font file paths or embedded font data. Browser/LAN clients do not enumerate their viewer machine;
  they keep manual entry and packaged fallback suggestions when this command is unavailable.
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
- `list_codex_accounts {}` -> `{ home: string, label: string }[]`.
  Owner desktop/LAN only. Lists existing local Codex home directories and saved selections;
  discovery does not read credentials, log in, start a harness or query plan entitlements.
- `save_project { project: Project }` -> Snapshot (empty id creates)
- `delete_project { id: string }` -> Snapshot (unassign chats; preserve their history and runtime)
- `post_project_board_note { projectId: string, text: string, requestId: string }` -> Snapshot.
  Owner desktop/LAN only; the `requestId` deduplicates retries. Requires the opt-in
  `settings.projectBoardEnabled` flag and an existing project. Notes are plain text,
  at most 2,000 bytes, and never start an agent turn.
- `set_task_project { taskId: string, projectId: string | null }` -> Snapshot

- `create_task { input: CreateTaskInput }` -> Task (rejects the internal Monitter Admin agent as a chat recipient)
- `fork_task { input: { sourceTaskId, throughMessageId, agentId, modelSettings? } }` -> Task (copies ordinary user/assistant text through the selected message into a new idle chat; no native session or automatic model turn)
- `plan_jev_route { agentId: string, prompt: string }` -> `JevRoutePlan` (native-owner only; requires an
  explicit non-internal harness opt-in and the Keychain-backed `TYPESAFE_API_KEY`; `JEV_API_KEY` is a legacy alias). It classifies a fresh prompt,
  writes an app-local trace containing a prompt fingerprint and typed decision/evidence, and does not create a
  task, invoke a provider, inspect a workspace, grant permissions, or expose the credential. LAN callers are
  rejected. A `human_review_required` result remains an advisory permission signal in the
  route card; it does not create a review step, grant authority, or change the task's
  existing approval boundary.
- `record_jev_route { taskId: string, traceId: string }` -> void (native-owner only). After a new
  chat is created, the desktop reads the already persisted route trace by UUID, checks its agent
  identity against the created task, and appends one
  idempotent `jevDecision` card to that task. The card shows the selected or recommended model tier,
  reasoning level, model-route confidence, application outcome, and available Jev cost/timing
  metadata. It contains no raw prompt or credential. Existing chats are not automatically rerouted.
  Live plans retain confidence for each typed judgment. Safe-auto eligibility uses the lower of
  the model-tier and reasoning-level confidences with a 0.30 floor. When the deterministic
  architecture/production rule raises the route to frontier/xhigh, eligibility instead uses the
  task-kind confidence because those model/effort answers were overridden. The lowest of all five
  judgments remains visible in expanded details but cannot veto an otherwise usable model
  choice. Older traces without per-question confidence retain the prior 0.50 overall gate.
  An explicit model choice still wins, and an unadvertised mapped model is never selected.
  For ACP safe-auto routing, a mapped model is first recorded as `modelStatus:"pending"`;
  the card reports `applied` only after the live session acknowledges the exact model option, or
  `already_current` when that option is active. Unsupported, rejected, and unconfirmed outcomes
  are shown without claiming the model was applied. Direct harness selection is recorded as
  `modelStatus:"selected"`; older route records without this field retain their historical display.
  For a new ACP chat in safe-auto mode, a qualifying Jev reasoning level remains pending until
  the live session advertises a `thought_level` select control. Monitter requests only the exact
  advertised level through `session/set_config_option` after any model selection and before the
  first prompt. An acknowledged choice is recorded as applied; a level already current is recorded
  separately. Missing controls, missing levels, and rejected requests keep the harness default and
  are shown as such on the route card. A timed-out setting is shown as unconfirmed and the turn
  fails without claiming the harness applied it. Jev never changes ACP permission controls.
- `plan_jev_command { query: string, candidates: JevCommandCandidate[] }` -> `JevCommandPlan`
  (native-owner only; requires the Keychain-backed `TYPESAFE_API_KEY`; `JEV_API_KEY` is a legacy alias). The caller supplies a bounded catalogue of
  currently enabled Cmd-P controls. Jev may select only one offered candidate ID and returns a typed confidence
  and bounded reason. The command records a fingerprinted audit trace without raw query/catalogue text and never
  executes a control. The UI must revalidate the candidate against its current enabled catalogue and require a
  second explicit selection before dispatch. LAN, mobile, and shared callers cannot invoke this command.
- `Agent.jevDecisionsEnabled?: boolean` opts a non-internal agent into Jev Decisions. Missing and false are
  disabled. This is separate from `jevRouting` and does not alter task routing. For an opted-in agent, the
  supported local harness adapter exposes a dedicated Jev MCP endpoint to that harness with the separate
  `jev_choose` and `jev_assess` tools and their decision guidance. The endpoint is local to the desktop runtime;
  its transport address is runtime-managed and is not a stable public URL. The add-on requires the native
  Keychain-backed `TYPESAFE_API_KEY` (`JEV_API_KEY` remains a legacy alias). It is delivered at the next
  newly launched session; saving the setting does not restart or reconfigure an existing session. It is available only to
  local Codex, Claude, and ACP harnesses. SSH and unsupported harness adapters do not receive it. The internal
  Monitter Admin agent is excluded. Jev tools may return bounded typed decisions and assessments only: they do
  not execute tools, create tasks, inspect files, modify data, change model routing, grant permissions, approve
  requests, or expand the harness's or Monitter's authority. Credentials and the endpoint are not exposed to
  LAN/controller/visitor clients or persisted in snapshots or workspace exports.
  The separate loopback broker issues a bearer grant for one running task and revokes it when that run ends.
  `jev_choose` accepts a state of at most 16 KiB, one question, and 2–16 distinct candidate IDs including
  `abstain`; it returns the selected ID, typed probabilities and confidence, or a fixed unavailable result.
  `jev_assess` accepts the same bounded state and one Noul yes/no question or a Score with 2–6 ordered levels;
  it returns typed probability or score data. Each call is advisory and the agent must recheck current evidence
  before using a result. Invalid or failed model answers never become tool authority. Best-effort local receipts
  record task ID, tool, outcome, model, latency, and token counts without state, question or candidate text.
- `refresh_mail_inbox { taskId: string, accountId: string, query: string }` -> JSON receipt.
  Native-owner only, absent from LAN/controller/visitor dispatch. Uses only Monitter's native
  Gmail connection assigned to the task's agent. The query argument is ignored; the native inbox
  searches the latest 100 unread inbox envelopes and updates that task/account inbox in place;
  absent messages move into local history.
  The durable batch records `nativeAccountId` and the verified account email. A missing connection,
  revoked assignment or provider error fails visibly. No harness turn or agent MCP is used.
  Jev receives only bounded envelopes in chunks of 10. Each card has five advisory Choice classifications plus
  `inboxFit { workRelated, requiresMyAction, excludedBulkOrAutomated }` Noul yes-probabilities
  (integer 0–100). The native inbox shows only Jev-classified active cards that pass the
  work/action/exclusion gate; provisional, fallback and older cards cannot pass it. Legacy agent-fed
  inboxes are hidden until replaced by a native refresh. Collaboration MCP no longer exposes
  `mail_triage_help`, `present_mail_batch` or `present_mail_detail`.
  A successful Jev result marks a low-importance message read only when action probability is at
  most 10%, bulk/automation probability is at least 95%, and the suggested action is archive/none.
  Failed or uncertain classifications never change Gmail.
- `request_mail_detail { taskId: string, mailId: string }` -> `{ status: "ready" }`.
  Native-owner only. The durable card must belong to the exact task. Monitter reads only that
  provider message through the recorded native account and checks assignment before/after retrieval.
  Legacy agent-fed cards require a native refresh before they can be opened. There is no agent
  fallback. Incomplete content is marked explicitly. Opening marks that Gmail message read via
  the native connector; existing read-only OAuth grants require reconnection for modify scope.
- `get_mail_detail { taskId: string, mailId: string }` -> `MailDetail | null`. Native-owner only.
  Returns a matching message from the bounded 30-minute process-local cache after checking native
  account access. Full bodies are never persisted in Snapshot, SQLite, compact UI projections,
  visitor sharing, or RunEvent detail. Optional HTML is rendered in an isolated reader without
  remote images or active content.
- `summarize_mail_thread { taskId: string, mailId: string }` -> `MailThreadSummary`.
  Native-owner only. Reads up to 50 messages in the selected Gmail thread, then returns a bounded
  source-excerpt digest from the latest eight; it does not persist full bodies.
- `get_model_catalog { target: { taskId?: string, agentId?: string, projectId?: string | null, codexHome?: string | null, refresh?: boolean } }` -> ModelCatalog.
  Agent targets work before the first turn. ACP catalogs are obtained from the saved launcher by
  initializing an isolated ACP connection and creating a model-free session; no user prompt or
  model request is sent. The advertised opaque model IDs are used exactly as returned. A bounded
  five-minute in-process cache distinguishes launcher, host and folder; `refresh` bypasses it. A harness that advertises
  no catalog reports that limitation visibly. Task targets use the live ACP session when present.
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
- `get_task_slash_commands { taskId: string }` -> `SlashCommand[]`. Owner desktop/LAN only.
  Native Codex tasks expose Monitter's protocol-backed Codex catalog once a native thread exists;
  resident ACP tasks expose the latest replacement snapshot advertised through ACP
  `available_commands_update`. ACP discovery is runtime state and is empty while disconnected.
- `execute_task_slash_command { taskId: string, command: string }` ->
  `{ effect: "sent" | "notice" | "openModel" | "refreshGoal", message?: string | null }`.
  Owner desktop/LAN only. The command name must exist in that task's current provider catalog.
  ACP commands are sent as the raw slash prompt; Codex commands use the app-server or the same
  authoritative Monitter services used by the corresponding UI, never an ordinary model prompt.
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
- `list_schedules {}` -> Snapshot (reads `schedules` and `scheduleRuns`)
- `save_schedule { input: ScheduleInput }` -> Snapshot (empty `id` creates; `preset` and `frequency` mutually exclusive at the input layer, `preset` wins; rejects unknown presets, invalid cron, missing or internal agents, unknown timezones)
- `delete_schedule { id: string }` -> Snapshot (removes schedule and its run log; tasks created by past fires remain in chat history)
- `run_schedule_now { id: string }` -> Snapshot (dispatches via the same overlap and mode-aware path as a timer fire; records a run with the same shape)
- `pause_schedule { id: string }` -> Snapshot (flips `enabled: false`; preserves run log and `lastFireAtMs`)
- `resume_schedule { id: string }` -> Snapshot (flips `enabled: true`; resets `consecutiveFailures` to 0)
- `list_chat_archive_rules {}` -> `{ rules: ChatArchiveRule[], projects: { id, name }[], checkIntervalSeconds, ageBasis }` (authenticated collaboration-MCP only; read-only)
- `save_chat_archive_rule { scope: "global" | "project", project_id?: string, older_than_days: integer, enabled?: boolean }` -> `{ rule: ChatArchiveRule, appliesOnNextCheck: boolean }` (authenticated collaboration-MCP only)
- `delete_chat_archive_rule { scope: "global" | "project", project_id?: string }` -> `{ removed: boolean }` (authenticated collaboration-MCP only)
- `archive_chats { scope: "global" | "project", project_id?: string, older_than_days: integer, dry_run?: boolean }` -> `ChatArchiveResult` (authenticated collaboration-MCP only)
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

## Built-in Mail plugin

Mail account setup is owner-desktop-only. `mail_get_config {}` returns
`MailConfig { revision, clientId, clientSecretConfigured, accounts }` with
`MailAccount { id, provider, email, label, agentIds, connectedAt }` metadata.
Credentials and account assignments live outside Snapshot in a separate macOS
Keychain vault; OAuth tokens never reach the renderer, harness environment,
managed MCP configuration, LAN, controller, visitor, or diagnostic output.

- `mail_configure_gmail { revision, clientId, clientSecret? }` saves the installed
  Google Desktop OAuth client; the secret is write-only. Returns `MailConfig`.
- `mail_start_connect {}` starts a bounded loopback PKCE flow and opens the system
  browser. Returns `MailConnectStart { attemptId, authorizationUrl, expiresAt }`.
- `mail_connect_status { attemptId }` returns
  `MailConnectStatus { status, error, account }`; status is `pending`, `connected`,
  `error`, or `cancelled`.
- `mail_cancel_connect { attemptId }` cancels a pending connection.
- `mail_set_account_agents { revision, accountId, agentIds }` saves exact per-account
  grants and returns `MailConfig`. Unknown, internal, SSH, unsupported provider,
  and AGY headless profiles reject. New accounts have no assigned agents.
- `mail_disconnect { revision, accountId }` removes the local connection and its
  grants, returning `MailConfig`.

A separate task-scoped `monitter_mail` Streamable HTTP MCP is attached on the next
harness launch only when a supported local user agent has an assigned account.
Local Codex, Claude, and ACP with HTTP MCP support use their existing managed
server transport. Managed server IDs cannot impersonate `monitter_mail`.
Each call verifies the running task, current agent eligibility and account grant;
revocations are also checked before releasing provider results. Task retirement
revokes its bearer token. Already-resident sessions need a fresh harness launch to
acquire newly enabled tools. Account grant changes apply immediately to sessions
that already carry Mail tools; removing access takes effect without a restart.

The provider-neutral catalogue is read-only: `mail_accounts`, `mail_search`,
`mail_read`, `mail_thread`. Gmail is the initial provider. `mail_search` accepts
`account_id`, native `query`, `limit` (default 20, max 50), opaque `page_token`,
`label_ids`, and `include_spam_trash`. Gmail operators belong in `query` and pass
through verbatim. `mail_read` takes 1–20 `message_ids`; `mail_thread` takes a
`thread_id` and `max_messages` (default 20, max 50). Both bound each message's body
with `max_body_chars` (default 20,000, max 50,000), report truncation, and expose
attachment metadata only. Mail content is untrusted data, never tool instructions.
Results intentionally enter the requesting agent's conversation; the plugin
itself does not persist message bodies. There are no send, modify, mark-read,
archive, delete, attachment-download, or account-administration tools.

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

## Environment & Secrets

Settings → Environment & Secrets is a native macOS-only vault. `list_environment_secrets {}`
returns only `{ revision, entries: [{ name, description, updatedAt }] }`.
`set_environment_secret { revision, name, value, description }` and
`delete_environment_secret { revision, name }` return the same redacted shape.
Values are stored together in the macOS Keychain and are never returned after a
write. These three commands are local Tauri commands only: they are absent from
the LAN dispatcher, controller and visitor command surfaces, and from `Snapshot`
and `Settings`.

Names use the ASCII environment-name form and reject reserved runtime names,
including `PATH`, `HOME`, `SHELL`, `TMPDIR`, `CODEX_HOME`, `MONITTER_*`, and
`OPENCODE_CONFIG_CONTENT`. Vault changes apply when a local user-agent harness
next launches through Codex app-server, Claude, OpenCode, Hermes, or ACP. They
do not restart existing sessions and never apply to SSH agents or Monitter Admin.

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

### Project coordination boards

Project boards are private coordination feeds projected as `#Project Name` under
Channels in the owner interface. Their records are not ordinary `Channel`
messages and are not included in the shared-visitor chat projection. The
persisted `Snapshot.projectBoardMessages` entries have a project ID, author,
optional agent/task IDs, Unix-millisecond timestamp, and monotonically
increasing per-project sequence. Turning the setting off hides boards and
rejects reads/posts without deleting notes; deleting a project hides its board
but preserves the locally stored notes for recovery. The paired mobile
controller shows these channels read-only.

The grant-scoped collaboration MCP adds `project_board_read {after_sequence?}`
and `project_board_post {request_id, text}`. The broker supplies the caller's
task; neither tool accepts a project ID. Calls require a running project task,
collaboration-enabled agent, and the board toggle. A read without a cursor
returns the latest 20 notes; a read with a cursor returns up to 50 newer notes
and reports whether more remain. The harness puts up to 10 recent notes in
every ordinary project turn prompt and instructs the agent to check again at
meaningful checkpoints. Board content is untrusted context, not permission or
a new user instruction. New project tasks with collaboration enabled also get
an initial system instruction to use Monitter MCP as the first stop for board
coordination, including an initial read and concise claims/updates. Existing
tasks still receive the per-turn board context when the setting is enabled.
Agents are not automatically woken by posts.

The owner can set `Settings.boardGuidanceText` as the global default and
`Project.boardGuidanceText` as an optional per-project override. Missing,
empty, or whitespace-only values inherit from the next scope, then use the
compiled built-in wording. Guidance is validated to 8000 characters and may
not contain control characters. Custom wording is passed as context and never
grants permission or overrides the user's request. Settings and project edits
apply to the next generated turn context and to each new MCP initialize
handshake; existing transcript system messages are retained as history.

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

### Task-scoped work plans via the collaboration MCP

The authenticated, active task may call `work_plan_start {request_id, title, items}`,
`work_plan_update {plan_id, item_id, status, note?}`, `work_plan_list {}`, and
`work_plan_close {plan_id, summary}`. The broker supplies the task identity;
tool arguments cannot target another task. A task has at most one active plan
and at most 64 retained plans. Each plan has 1–32 items. Start retries with the
same request ID and payload return the existing plan. Updates are idempotent.
Close requires every item to be `completed`, `blocked`, or `skipped`; it cannot
silently discard `pending` or `in_progress` work. The durable `Snapshot.workPlans`
projection uses camelCase fields and Unix millisecond timestamps. Plan history
is removed when its task is permanently deleted, not when archived. The owner
Task detail sidebar displays the selected task's current plan directly below
its Git branch, including item states and notes; prior plans are collapsible.
Shared visitors do not receive the work-plan projection.

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
Composer text is sent and stored with its newline characters intact. Pasting clipboard content
that contains both text and files inserts the text and attaches the files. User chat bubbles render
single newlines as visible line breaks, including optimistic and shared-chat messages; assistant
Markdown keeps its normal parsing behavior.
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
local Codex app-server task sends `turn/steer` using its current thread and turn IDs. An ACP task may
also steer only after its initialize response explicitly advertises a reviewed v1 namespaced method:
Mcode's `_meta["minimax-code/extensions"]` `mcode/session/steer`, or Mona's
`_meta["mona/extensions"]` `mona/session/steer`. Monitter sends the advertised method with the live ACP
session ID, expected prompt-turn ID, and a client request ID. It accepts only a correlated
`mode: "steered"` reply while that same prompt remains active. Monitter retains the durable `sending`
record as a user bubble in its chronological transcript with a `Steering` tag until the harness
acknowledges the matching turn, then atomically replaces it with the persisted user message. A
temporarily-not-ready harness is retried only while that exact prompt remains active. An unavailable,
stale, rejected, or unsteerable turn returns the record to FIFO; its transcript bubble changes to
`Queued` and a visible fallback event is recorded, without duplicating it in the queue dock. Other CLI
adapters safely queue. The next queued item starts only after its owned native run releases. A
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

Settings include optional `interfaceFont`, `chatFont`, and `terminalFont` family names. Empty or missing values use IBM Plex Sans for interface/chat and IBM Plex Mono for terminals. Installed custom family names are supported with fallback fonts; changes apply to existing terminals without restarting sessions. The desktop Typography picker asks `list_system_fonts` for local OS family-name suggestions, keeps manual entry available, and falls back to the packaged suggestion list if enumeration fails.

`interfaceFontWeight`, `chatFontWeight`, and `terminalFontWeight` are saved independently as numeric weights 300, 400, 500, 600, or 700, each defaulting to 400 for older snapshots. The Typography sliders preview changes while dragged and save on release. Interface weight changes ordinary UI text without removing explicit heading/bold emphasis; chat weight applies to messages and the composer; terminal weight updates xterm's normal and bold renderers in existing sessions. A selected font may map an unavailable weight to its nearest installed style.

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
agent reply. Other status diagnostics remain in Task detail, and visitor sharing continues to
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

### Multiple local Codex accounts

`Agent.codexHome` is an optional absolute path to an existing local Codex home.
Null selects Monitter's inherited `CODEX_HOME`, falling back to `$HOME/.codex`.
Explicit account homes are supported only for local Codex agents. Monitter validates
and canonicalizes the selected directory without copying credentials or rewriting
Codex configuration. Directory labels identify a local setup, not verified account
identity or subscription status.

Every newly created local Codex task snapshots its resolved home in `Task.codexHome`,
including channel and delegated chats. Changing an agent's account affects future
chats. Existing chats retain their home for launches, resume, model and goal probes,
and native-session file lookup. On state load, legacy local Codex tasks without this
field are pinned to the inherited home when that directory exists; a missing home
does not prevent state from opening. SSH tasks keep the remote host's account and
cannot select a local Codex home. Session ownership distinguishes Codex homes. Each child process gets
its own `CODEX_HOME`; Monitter never changes its process-global environment.

Subscription allowance sources carry optional `codexHome` and `accountLabel` fields.
Each discovered/saved local Codex home has an independent quota source, including
failures; caches and UI identities distinguish those homes. These are separate
account allowances, not pooled credits. Model-catalog caches also distinguish homes.
Account paths are owner configuration and are omitted from shared-visitor projections.


ACP is a generic transport (`provider: "acp"`), not an agent-brand enum. Its
optional `Agent.acp` / `Task.acp` launcher contains an executable `command` and
an exact string-array `args`, never a shell command. Older records omit it.
New ACP tasks copy the agent's launcher; editing the agent does not retarget an
existing chat. Non-ACP tasks do not carry an ACP launcher. Session ownership
distinguishes ACP launch configurations. ACP normally uses `harness-configured` permissions.
An explicit `yolo` selection requests the live session's advertised `bypassPermissions` mode (including
Mcode's process-scoped `permissionMode` selector). If that exact option is absent, each live
`session/request_permission` may receive only the agent's advertised opaque `allow_once` option.
Malformed, missing, duplicate, or broader-only options are cancelled; Monitter never selects an
`allow_always` option. This changes only the agent process's own confirmation policy: ACP remains a
transport, not an OS sandbox, and it does not widen Monitter's native, LAN, filesystem, or collaboration
permissions.

An ACP harness that negotiates Monitter's `jev_routing` extension may emit a bounded
`session/update` with `sessionUpdate: "router_trace"`. Monitter validates a structured trace no
larger than 64 KiB and stores one visible status event containing both the requested and actual
model/effort plus the application outcome. A failed application is labelled as a rollback and does
not claim the requested model ran. Malformed or oversized traces create a bounded diagnostic error
without failing or replaying the model turn. Raw prompts and credentials are never accepted as part
of this extension payload.

When an ACP `session/prompt` result includes authoritative response attribution, Monitter may
persist it on the completed assistant `Message.responseMetadata`. The bounded metadata contains
the actual model, input/output token counts, and Jev's routing rationale and outcome; it is attached
only at the matching prompt-completion boundary and never inferred from the task's configured
model. Older messages and ACP agents that omit these result fields have no response metadata. The
transcript shows only the actual model and an information control below the assistant bubble;
token and routing details remain inside that disclosure. Shared and paired-mobile task transcripts
receive the same safe projection. Prompt text, provider credentials, and unrecognized routing
fields are never copied into the message.

Agent settings offer searchable presets and a custom ACP launcher. The catalog
is convenience metadata, not a restriction on which compatible executables can
be used. Bridges (including Pi ACP) are labelled separately from native ACP.
The local Gemini preset uses Monitter's bundled `monitter-agy-acp` bridge over
the installed, authenticated `agy` CLI's resident headless stream. The bridge
is local only, requires Node.js, retains the AGY conversation ID for ACP
`session/load`, and does not advertise HTTP MCP or interactive approval support.
New bridge profiles have collaboration and Jev Decisions MCP disabled. The
normal permission level maps to AGY `accept-edits`; explicit Full access maps
to AGY's skip-permissions flag. The catalog also offers Google's separate
Antigravity ACP server (`agy_acp_server.par`, no arguments), which has its own
authentication flow and supports richer ACP capabilities. Startup upgrades
only local agent profiles with the exact old `gemini --acp` preset when the
bundled bridge, AGY, and Node.js are available, clears their old model
selection, and disables unsupported MCP opt-ins. Existing chats retain their
pinned launcher and native session; start a new chat for AGY. Custom launchers
and SSH agent profiles are not rewritten.
Pi's native `--mode rpc` must never be treated as ACP. Existing OpenCode tasks
remain on their prior adapter; OpenCode ACP is selected explicitly for new chats.
Launcher paths/arguments remain owner-only and are omitted from shared-visitor
agent and task projections.

Codex chats use one owned `codex app-server --listen stdio://` process per task over private stdin/stdout
JSON-RPC. Initialize once, create/resume the saved native thread, and start each user turn on that
connection. Local hosts launch the process directly; SSH hosts carry the same full-duplex protocol
through an owned `ssh -T` connection. No HTTP/WebSocket listener is exposed. SSH uses the remote
host's CLI, authentication, configuration and working folder, without copying local credentials.
It does not use the one-shot `exec --json` supervisor for chat turns.
Codex app-server bounds initialize to 20 seconds, thread start/resume to 150 seconds, and each
turn start to 20 seconds with independent clocks; notifications never extend a pending request.
Resume requests exclude stored native turns because Monitter preserves the local transcript.
Resume is explicit by native session ID. Users may attach an existing idle native session by ID; old
transcript import and live takeover of another Desktop/TUI process are not implied.
Codex replies use item text deltas and authoritative completed items. The same durable
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
are not imported into the chat or logged. New Hermes Agent profiles launch the installed `hermes acp`
server through the ordinary ACP transport, including its advertised session configuration. Existing
native Hermes profiles remain on the TUI gateway with the Python bridge: session.create/resume,
length-framed multiline prompt.submit, durable stored_session_id, normalized JSONL events, explicit
denial of interactive requests, and owned child cleanup. An explicit native Hermes `provider/model`
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
If a load/resume response explicitly returns an identity, it must match the requested saved
session exactly. A changed, malformed, or conflicting identity fails before the transport is
published as ready or receives a prompt. The saved ID and transcript remain unchanged. A load
response that omits identity legitimately reuses the requested saved ID.

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
OpenCode and legacy native Hermes reject `yolo`: OpenCode's `--auto` still respects explicit denials,
and the Hermes bridge has no verified per-invocation bypass. Hermes ACP follows the ordinary ACP rule:
it accepts `yolo` only after the live session explicitly advertises `bypassPermissions`. Other ACP agents
also accept it only after that advertisement; other providers require `harness-configured`;
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

Current interactive approval transports include local and SSH Codex app-server, legacy Hermes' local full-duplex gateway and Claude's local
stream-json host protocol. A Claude `can_use_tool` request creates one durable desktop approval and the
chosen one-time allow or deny is returned to that exact control request ID with the provider's original
tool input preserved. SSH Hermes and SSH Claude remain safe-deny/unsupported until their remote
supervisor supports the same persistent full-duplex channel. OpenCode `run` does not
show enabled interactive approval buttons. No auto-resubmission after
errors.

The resident Monitter Admin owns a pool of at most three hidden chats for interface work
(currently `autoname`). Bootstrap still creates exactly one internal agent named "Monitter Admin"
on `Service::open`, exposes its configuration in Settings → Agents, and hides it from conversation
surfaces. Each pool chat is created lazily, never archived, and never projected into ordinary chat
lists. Requests reuse idle resident transports and may run concurrently on three distinct chats.
Further requests wait silently in a process-local FIFO queue until a slot is released; queue
capacity does not produce a busy error. The 45-second execution deadline starts after admission,
so waiting for a slot does not consume the turn budget. All waits happen off the UI thread.

Each transport follows the five-minute idle retirement policy. The next request restores its
saved native session silently. App quit, admin reconfiguration, or transport failure retires the
corresponding resident controls. Shutdown wakes queued callers; neither queued nor in-flight
prompts are persisted or replayed after restart. Real configuration, provider, or execution-timeout
errors remain visible. The admin's saved provider, model, host, cwd, and sandbox are used as-is,
with no mini-model selection, provider fallback, or automatic retry.

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
SSH Codex uses a bounded remote supervisor that keeps stdin open for JSON-RPC and terminates its
owned process group on transport EOF, cancellation or process exit. Collaboration uses an owned
remote-loopback reverse forward to the task-scoped HTTP MCP broker. Setup failures include bounded
SSH diagnostics; strict host-key failures require fixing the saved host/trust configuration, not an
automatic trust bypass. Disconnects expire approvals and never replay an uncertain turn.
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
`jevDecision` records a bounded, state-free Jev decision card: the question, readable candidate or
score-level labels, validated typed answer, confidence/probability distribution, and available
provider/model, timestamp, latency, token, and cost metadata. It never stores the input state or
API credential. The live projection keeps a compact parseable summary; the full record is available
through the task-event detail endpoint.
The same event kind also displays a new-chat `jev_route` record sourced from the native route trace.
Its confidence is the lowest of Jev's five routing judgments, so its compact bar is a confidence
indicator rather than a choice distribution. It distinguishes a model selected for the chat from a
Jev-only recommendation; permission signals remain advisory.
New route plans retain `policyVersion`, `questionSchemaVersion`, an optional classifier `modelVersion`,
and optional full `question_probabilities` maps for all five choices. Legacy traces remain readable
with unspecified versions; missing probabilities are never fabricated. Offline evaluation compares
explicitly imported outcomes by case and repeat IDs with caller-supplied evidence references and separates replayed policy agreement from
observed quality, cost and latency. Synthetic fixtures establish code behavior, not live model quality.
The ACP route receipt also distinguishes a requested model from an acknowledged model change,
an already active model, and unsupported, rejected, or unconfirmed model requests.
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

Typing `/` opens one filtered, keyboard-accessible command dock. It is 10px narrower than the
composer and rises from behind it, matching the goal dock's visual ownership. Every entry includes
its source: Monitter actions use the Monitter command mark and provider commands use that provider's
icon. These app actions remain available in applicable contexts: `/new` and `/clear` reset the
active chat's provider context while preserving its visible transcript. In an active chat,
`/new <title>` performs that reset first and then renames the pane to the supplied title; a failed
reset leaves the title unchanged. Outside an active chat `/new` opens an independent draft.
`/settings` opens preferences, `/project` selects the chat's project, and `/stop` cancels a running
task. Task-specific actions only appear in applicable contexts.
`/rename` names the current chat or channel from its recent messages. Controls → Auto-name current pane also names an active terminal from a bounded recent-output buffer; it is never written to that shell. Naming routes through the resident Monitter Admin agent (see Internal agent) on its saved provider, model, host, cwd, and sandbox. The admin uses no Spark/Luna/Mini mini-model selection, no provider fallback, and no persisted admin prompt/reply. Up to three requests run concurrently; further requests queue silently in FIFO order. The execution is bounded to 45 seconds after admission; the final target mutation (task title, channel name, or terminal rename) is reported through `monitter:changed` exactly like any other rename.
`/terminal` opens a terminal tab in the current host and folder; `/terminal <command>` runs that shell command in the new interactive tab (for example `/terminal npm build`). The command is written to the PTY after the shell starts, so the tab stays interactive when it finishes.
Selection supports arrows, Enter, Escape and clicking. IME composition does not select an action.

ACP has no request for listing commands. Instead, a resident agent may send ACP
`session/update` with `available_commands_update`; Monitter validates and bounds that replacement
snapshot, displays it with the configured provider icon, and invokes a selected command by sending
the exact `/name arguments` text in `session/prompt`. A reconnect starts with an empty catalog and
cannot reuse stale commands from a retired transport.

Native Codex has no app-server command-catalog endpoint, so Monitter owns a conservative static
catalog whose entries all have explicit implementations: `/compact` uses `thread/compact/start`,
`/review [instructions]` uses `review/start`, `/goal [objective|clear|pause|resume]` uses the native
goal API, `/model` opens the task model picker, `/usage` reads the authoritative allowance source,
`/status` reports the persisted runtime selection, and `/skills` plus `/mcp` issue read-only
app-server list queries. `/compact` and `/review` enter the normal task lifecycle and transcript;
informational commands return a bounded local notice and do not create fake provider messages.

Unknown commands and invalid arguments produce a visible explanation without sending ordinary
model text. The Send button and keyboard use the same dispatch rule. `//` explicitly escapes a
literal leading slash; paths such as `/path/to/file` remain ordinary messages.

## Internal agent

`Agent.internal` is an optional boolean that defaults to `false`. The bootstrap migration creates
exactly one internal agent on `Service::open`, named "Monitter Admin"; that record is created only by
the bootstrap, is never offered for creation by ordinary `save_agent` callers, and cannot be deleted.
The existing admin record is selectable in Settings → Agents so its provider, model, host, folder,
and permissions can be changed. The internal agent and its up to three resident tasks are hidden from
conversation surfaces:

- Sidebars, the monitter menu, the Agent directory, and Cmd-K / Cmd-P palettes list user agents
  only; the internal record is omitted from those projections, search results, and pickers.
- New chat, channel invite, channel membership, and channel recipient pickers exclude the internal
  agent. `create_task` rejects it as a chat recipient; `set_channel_membership` is a no-op for it.
- Extensions MCP/skill eligible-agent sets, collaboration discovery, and the resident routing
  eligible-agent set all filter out the internal record before iteration.
- Archived chats, the share control, mobile projections, and paired-mobile / controller replies
  never surface the internal agent or its tasks. Visitor snapshots exclude them by construction.
- Task detail, the right sidebar, and any agent-attributed chat metadata skip the internal record.

Editing the existing admin via `save_agent` preserves `internal: true`, the canonical "Monitter
Admin" name, and `collaborationEnabled: false`. Generic callers cannot create a new internal record
or clear an existing one. When the admin's configured provider, model, host, cwd, or sandbox changes
through ordinary agent edits, the next `autoname` runs safely replace the resident pool tasks/sessions so
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
`agentThreadId` = its ACP `subagentSessionId`, optionally including model and reasoning effort) and
`subagent_state_update` (`completed`, `failed`, `disconnected`, `cancelled`, with optional terminal
detail) notifications on the parent session. An ACP adapter that receives only a native swarm snapshot
must project every non-root member through those same notifications, rather than silently keeping its
background workers private. An agent that does not advertise support keeps folding that activity into the
root transcript as before. Monitter delegations contribute
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
- `list_agents`: search configured, published agents by profile. Optional `active_only: true`
  limits discovery to agents whose task has a currently active runtime turn, using the live run
  registry rather than persisted task status. Results expose `active`, `active_task_ids`, and the
  newest `current_task_id`; omitting the option keeps inactive profiles available for delegation.
  Each result also includes `defaultModel` (saved model, or the discovered harness default when
  available), compact `models`, and `modelCatalog` metadata
  (`source`, `warning`, and `complete`). Compact model entries use the advertised catalog's
  camelCase fields (`id`, `name`, `description`, `reasoningEfforts`, `defaultEffort`,
  `supportsFast`, `fastDescription`) plus `power: { tier, source }` and `powerTiers` (all operator
  tier mappings). Directory lookup uses already-cached catalogs (at most 16 models per profile)
  and never starts a model probe; missing catalog data yields an empty
  model list with `complete: false` and points callers to `list_agent_models`. Power tiers are
  `fast`, `balanced`, `strong`, `frontier`, or `unknown`; sources are
  `operator-configured` or `unknown`. Unknown power means no unique operator mapping is available
  (none or multiple); `powerTiers` preserves all mappings.
- `list_agent_models { to_agent_id, refresh? }`: read the recipient's full advertised model and
  reasoning-effort catalog, performing targeted discovery when needed. `refresh: true` bypasses
  the short-lived catalog cache. Depending on provider, discovery may initialize a local ACP
  session without sending a prompt. Catalog `reasoningEfforts` lists exact effort IDs accepted
  for that model. An empty list in a complete
  catalog means no effort choices are selectable through Monitter. This does not assert that the
  underlying model cannot think: some harnesses control effort internally or do not expose it.
  Consult `modelCatalog.warning` for provider limitations. `complete` describes model-ID menu
  coverage only, not completeness of the provider's private capabilities.
- `delegate_task { to_agent_id, title, message, request_id, model?, reasoning_effort? }`: create a
  child chat, inherit its parent's project, apply the recipient's saved host/folder/permissions,
  and durably queue the supplied brief. Retries retain the first selection; changed title, model,
  or effort under the same request ID reject. Older records without saved title retain legacy
  retry behavior. Omit `model` to retain the recipient's saved model default.
  Set `model: "auto"` to ask the configured Jev service to select from a bounded shortlist of
  advertised models, preserving the configured default and operator tier entries in the shortlist.
  It falls back to the recipient's saved default when Jev is unavailable, abstains, is uncertain,
  returns invalid evidence, or the input is sensitive or exceeds selection limits. The explicit
  `auto` request uses this delegation policy independently of the chat composer's routing toggle.
  Otherwise `model` must be
  an exact model ID from that recipient's advertised catalog and unavailable choices are rejected.
  `reasoning_effort` is optional, must be an exact effort ID advertised for the explicitly selected
  model, and is rejected when model is omitted or set to `"auto"`. Model selection does not grant
  permissions or change the saved host, folder, sandbox, or provider policy. Results and the durable
  collaboration row include `modelSelection`: requested title/model/effort, resolved model/effort,
  selection `source` (`explicit`, `jev`, or `configured_default`) and a bounded reason. A null
  resolved model means the harness's native default is retained; it is not a claimed model turn.
  Routed subagent sessions also expose the child task's configured model and effort.
- `send_message`: send a brief to a peer, optionally using a directly connected collaboration chat.
- `get_task_result`, `wait_for_task`: read the actual result and incoming peer messages.
- `list_messages`: deliver queued messages to this active turn and return its durable inbox.
- `cancel_delegation`: cancel an active delegation owned by this task.
- `terminal_run`: open an interactive Monitter terminal tab and run a shell command in it.
  Requires the agent's independent `terminalExecutionEnabled` opt-in (default false, including
  older saved agents), a live running turn, and a one-time native **Host terminal** approval
  of the exact command. Read-only or unknown task sandbox modes cannot use this capability.
  `command` is bounded to 4096 bytes without NUL; optional `cwd` must equal the task's saved
  folder. The immutable task host/folder is used even if its agent profile has changed.
  The approval explicitly covers ordinary host-shell execution outside the harness sandbox;
  it cannot reuse provider tool rules or a session/always grant, including in YOLO mode.
  At most one terminal approval may be pending per task, and it expires after two minutes,
  cancellation, turn replacement, or capability revocation. Ownership and turn identity are
  rechecked before execution. The shell stays interactive after the approved command finishes.
  The result includes `approvalRequestId` to correlate its native approval receipt.

The collaboration MCP cannot populate or deliver inbox content. The native inbox commands above
own Gmail search and detail retrieval. Native envelope batches retain the existing background Jev
workflow: one request with five typed Choice questions per message for importance, intent, reply,
owner and suggested action. Per-dimension confidence and provider/latency/usage evidence remain
visible. Empty results skip Jev, stale enrichment is discarded, and unavailable classification
retains an explicit low-confidence local fallback. Email and model output remain untrusted data.

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
or disabling other configured MCP servers. Hermes ACP receives the same scoped MCP configuration as
other compatible local ACP servers when it advertises support; legacy Hermes profiles retain their
existing gateway behavior.

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

The inbox uses Monitter's assigned native Gmail connector directly; it does not call a harness Gmail
plugin or the collaboration presentation tools. Email text is always data rather than instruction.
Jev classification cannot widen mailbox, harness, filesystem, or approval authority. The inbox
exposes no compose/send/reply/forward, archive, label, delete, attachment download, or background
mailbox polling action. `MailBatch` is included in owner desktop/LAN snapshots, but the explicit
shared-visitor projection omits it; full message viewing remains native-owner only.

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

## Harness handoff

`handoff_task { sourceTaskId, agentId, note? }` creates a new task on the selected, different
agent and immediately sends it a bounded continuation brief. A handoff is available voluntarily
and may be surfaced when a provider reports exhausted allowance. It is not a portable native-session
transfer: the source native session, credentials, running transport, queued messages and pending or
remembered approvals stay with the source task. The source must be non-running, non-archived and have
no queued messages or pending approval. The brief contains only recent ordinary user/assistant text,
the source provider, working folder and optional user note; it deliberately excludes raw provider state,
attachments, approval data and secrets. The target receives fresh permissions and is linked to the
source through `parentTaskId` for provenance.

The message header aligns its timestamp at the right and offers Copy, Reply and Fork beside
the author. Copy writes only displayed message text to the clipboard. Reply stores that text
as a normal Markdown attachment in the current task's working folder and places it in the
composer, without sending. Fork selects an agent and optional model, then calls `fork_task`.
The fork copies up to 2,000 visible user/assistant messages or 8 MiB of text, preserving
their order and timestamps but not file attachments, diagnostic/tool events, response usage,
native provider state, queued messages, approvals, or credentials. It adds a visible fork
divider, stays idle until the user sends, and links to the source with `parentTaskId`.
On that first send only, a bounded latest-32-message text excerpt (2,500 characters per
message) accompanies the new request so the new harness can use the copied history as
context. The modal discloses that limit and the fresh-session boundary. An unfinished
streaming message cannot be the cutoff.

## Pane layouts and navigation

The Controls command palette offers one pane, two, three or four columns, and 2 × 2, 3 × 2 or
4 × 2 grids. Each pane owns its tabs, selected
chat/channel, drafts and right sidebar. Dividers resize with pointer dragging or arrow keys. Tabs move
between panes; dropping toward a pane edge previews and creates a split, up to eight panes. Collapsing
the layout merges tabs and preserves drafts and uploaded references. Layout changes wait while a send
acknowledgement or attachment upload is pending. Pane layout and unsent drafts are window-local.

For a non-empty Cmd-P query, the native desktop may offer `Interpret with Jev`. Selecting it sends the
query and only the currently enabled, bounded control catalogue to `plan_jev_command`. The returned item is
shown as a proposal with confidence and reason; interpretation itself has no side effect. Selecting that
proposal is a separate confirmation, after which the UI revalidates and uses the existing control dispatcher.
Changing the query, closing the palette, or making the proposed control unavailable invalidates the proposal.

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

The main sidebar can collapse while retaining its active Agents, Projects or Activity view. The
collapsed Agents rail shows agent avatars with chat popovers, Projects shows project icons with chat
popovers, and Activity shows recent chat controls. Its client-local mini-tab row selects the view;
non-Agent chat rows include the owning agent's avatar. The sidebar participates in keyboard pane
selection: moving left from the leftmost content pane selects the sidebar, moving right returns to
content, and pane cycling includes the sidebar. Arrow Up/Down and Home/End move through visible
sidebar controls, Arrow Left/Right changes the focused view tab or collapses and expands a focused
group, and Enter/Space activates the focused control. The selected sidebar has the active pane
indicator; its focused control remains visible even when the indicator setting is off. Chat archive
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

Voice messages are recorded by the owner desktop or owner LAN browser as 16 kHz mono PCM16 WAV, with a maximum
duration of 90 seconds. `transcribe_voice_message { audioBase64: string }` returns
`{ text: string }` and invokes the locally installed whisper.cpp `whisper-cli` with a local
model with automatic language detection. Install whisper.cpp with Homebrew (`brew install
whisper-cpp`) and obtain the multilingual `ggml-base.bin` model from the
[whisper.cpp model repository](https://github.com/ggml-org/whisper.cpp/tree/master/models), then
place it at the default path below. Configure nonstandard locations with `MONITTER_WHISPER_CLI` and
`MONITTER_WHISPER_MODEL`; defaults discover `whisper-cli` on PATH/Homebrew and look for
`~/Library/Application Support/Monitter/whisper/models/ggml-base.bin` (or standard whisper.cpp
model locations). Recording runs on the user's device and transcription runs on the desktop Mac. The frontend adds the returned text
to the composer, and normal message delivery sends that text without attaching or retaining the WAV.
Transcription accepts only bounded, valid PCM16 WAV input; temporary audio and output files are removed after
the command. First-run Whisper startup can spend additional time compiling local Metal kernels;
the native process has a five-minute hard timeout. Errors and process output never contain audio content.
The transcription command is available to the native owner, the owner LAN view,
and paired phones through its separately approved encrypted owner relay.
The native macOS owner can set `Settings.voiceTranscriptionEngine` to `"apple"`
instead of its `"whisper"` default. In that mode the desktop calls native-only
`transcribe_voice_message_apple { audioBase64: string }` with the same bounded WAV
input and `{ text: string }` output. Apple's Speech framework is required to use
on-device recognition for the Mac's current locale; unsupported locales, denied
speech permission, and recognition failures return visible errors without silently
sending audio elsewhere. The Apple command is unavailable to LAN and paired
clients; their `transcribe_voice_message` requests continue to use desktop Whisper.
Both commands delete their temporary audio and send only editable text to the
composer and eventual agent message.
The mobile shells grant browser microphone capture only to their bundled owner
page. Shared visitors cannot invoke the owner command. It does not store the WAV or create an
attachment ID. The separate `read_attachment_audio { attachmentId }` reader applies only
to an already registered WAV attachment, validates MIME type, format, and duration, and
remains desktop-only. The macOS bundle declares the microphone and speech-recognition
purpose strings and the audio-input hardened-runtime entitlement required for recording.
Setup and privacy details are in `docs/VOICE_MESSAGES.md`.

Mobile on-device transcription is a separate shell-to-bundled-page path. The
iPhone's `monitterSpeech` WKScriptMessageHandler accepts only paired main-frame
requests from `http://127.0.0.1:31977`; Android's `monitterSpeechAndroid`
WebView listener accepts only its bundled main frame at
`https://appassets.androidplatform.net`. Both accept `{ action: "status" | "start" |
"stop" | "cancel", sessionId: string, locale?: string }` and return a
`monitter:native-speech` CustomEvent whose detail contains `sessionId`, `type`
(`availability`, `started`, `level`, `partial`, `final`, `error`, or `stopped`), and the
applicable `available`, `level`, `text`, or `error` field. On iOS, `level` is a
throttled normalized 0–1 microphone amplitude for local UI animation only;
it is scoped to the active recognition session and never sent to the desktop.
Status checks do not request
microphone permission. A session lasts at most 90 seconds of capture. Only a
final transcript enters an editable draft; neither native shell sends audio to
the desktop, another speech server, an attachment, or a harness prompt. Paired
phone Settings stores a per-device `desktop`/`on-device` preference, defaulting
to the existing desktop path. Android's bundled `/mobile` route bootstraps the
same separately approved owner workspace as iPhone; `/mobile?controller=1`
retains the earlier limited controller. Android local recognition is shown only
where an on-device recognizer exists; unsupported devices keep text input and
can choose the desktop path. No automatic recognition fallback sends audio elsewhere.

The paired iOS owner page also exposes an explicitly activated foreground
Companion Mode, named Mo. It uses the existing `monitterSpeech` recognition session and
the iOS on-device recognizer to detect the spoken phrase “Hey Mo” (also
accepting the common transcription “Hey Moe”) and capture the command that
follows. This is ordinary foreground speech recognition, not a dedicated
low-power keyword model; the microphone is not continuously active before the
user enables listening, and audio is not uploaded for recognition. The
selected chat ID is captured when listening starts; changing chats stops the
session. Native results revise the entire utterance: a revised wake phrase alone
never becomes a message, and an unrelated revision discards the earlier command.
“A Mo” is stripped only after an actual wake phrase has already been recognized;
it cannot activate listening by itself. A finalized wake-only utterance starts
a separate command capture session. Mo's tap area toggles capture on and off
and fills at least half the idle portrait viewport's height; the visible face
is 60% of that area's width and height and centered within it.
Mo blinks and floats gently at rest. During iOS speech capture, the face and
mouth react to live microphone level, with motion suppressed by the system
reduced-motion preference.
The Hey Mo and optional pause-to-send toggles sit below it. The idle companion
has no separate talk button, empty textarea or Send control.
Captured voice text opens an outgoing review bubble with explicit Edit and Send
controls. Auto-send is off by default.
When enabled, recognition continues across short utterance boundaries and sends
only after eleven seconds without new speech. New speech restarts that interval;
duplicate partial/final text does not. An explicit Stop always produces a draft
for review. Existing unsent draft text also requires explicit Send, even when
auto-send is enabled. Changing chats, editing, or disconnecting cancels capture
and prevents any pending automatic send.
Recognition cannot resolve approval requests or submit approval inputs; these
continue to use their existing explicit controls. Counts for pending requests
in other chats remain visible.

Completed desktop replies appear as dismissible speech-bubble popups over Mo,
with source-chat labels and scrollable Markdown. Opening Companion Mode shows
the latest selected-chat reply once; newly completed replies from eligible chats
queue in arrival order. Streaming, interrupted, commentary and blank messages
are excluded. Message IDs deduplicate repeated snapshots and reconnections
during that visit. A popup never changes the voice target; switching to its chat
requires an explicit action. The face and outgoing review controls remain reachable.

Companion activation uses the separate iOS-only
`webkit.messageHandlers.monitterCompanion.postMessage({ active: boolean })`
handler. It accepts only the paired main frame at `http://127.0.0.1:31977`,
with the same pairing and trusted-origin checks as `monitterSpeech`; the
speech command/event contract above is unchanged. Native code keeps the screen
awake while Companion Mode is visible and the app is foregrounded. Leaving the
foreground, an audio interruption or route loss, page navigation/closure,
Companion Mode closure, or desktop disconnect cancels active microphone
capture. Start and finalization waits are bounded and failures remain visible;
permission or availability errors require the user to re-enable listening.
Send failures do not cause an automatic replay of a possibly delivered request.
See `docs/IOS_COMPANION.md` for build checks and physical-device acceptance.

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
`config/read` and paginated `model/list`. OpenCode's legacy transport reads provider-qualified
IDs from `opencode models`. ACP agents can advertise models during an isolated session setup
before the first chat turn, including in Agent settings. The picker uses the exact advertised IDs
and exposes only supported reasoning levels and Fast capability. Harnesses without model
discovery retain their configured models and report the limitation. Catalog failures remain visible.

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
Pointer or keyboard focus immediately marks the receiving pane or sidebar active; its opacity is always 1.


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

## Task detail and timeline

The right sidebar separates Task detail, Timeline, and (when available) Git changes. Task detail keeps
agent identity and settings, adds a compact branch/staged/changed/untracked summary from the existing
Git status poll, displays the selected task's agent work plan beneath the Git branch, and lists
running Monitter tasks and terminals in the same host/folder. This list is labelled Tracked
processes: it does not claim to enumerate arbitrary OS processes or undisclosed jobs
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

`Settings.activityAnimation` is an owner preference shared by desktop, LAN, and paired mobile views. It defaults to `"sparkles"`; `"grid"` selects a pulsating accent grid and `"matrix"` selects a changing-letter stream. It only affects decorative indicators for running tasks. Existing or unknown settings values are rejected on save, while the motion preference and operating-system reduced-motion setting continue to limit animation.


`autoname { target: { taskId?: string, channelId?: string, terminalId?: string, content?: string } }` returns Snapshot. Exactly one target is required; terminal text comes from a bounded, control-sequence-scrubbed client buffer. Chat/channel context uses the last 12 messages, bounded to 12,000 characters. The naming request is a blocking resident turn on the resident Monitter Admin agent: `Service::send_admin_turn(prompt)` → `AdminTurnBroker` → the saved Codex app-server / Claude stream-json / ACP transport selected by the admin's configured provider. The admin's configured provider, model, host, cwd, and sandbox are used exactly as saved — there is no Spark/Luna/Mini mini-model selection, no provider fallback, and no automatic retry. Execution is bounded to 45 seconds after a pool slot is acquired; on expiry the owned resident control is terminated. Up to three admin requests run on separate hidden chats. Additional callers queue silently in FIFO order, with no capacity error and no execution timeout while waiting. Every completion, failure, or timeout releases its slot for the next caller. Shutdown wakes pending callers without replaying their requests. Internal prompts, replies, and request IDs are runtime-only and never enter `Snapshot.messages`, `RunEvent`s, `get_task_events` pages, compact LAN snapshots, or shared/visitor projections. The saved native session id on the internal task is persisted so a restart resumes the saved native session; an interrupted, timed-out, or failed request is never replayed. Titles are bounded to 60 characters. Terminal names update the existing session and are refreshed into the tab runtime. The final target mutation is reported through `monitter:changed` exactly as for ordinary renames. Use /rename in chat or find Auto-name current pane in the Controls palette (Cmd-P on macOS) for a terminal; the shell never receives the command.


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

The desktop app owns an internal LAN server. By default it generates a persistent,
per-install local root CA in the app data directory and serves LAN peers over
HTTPS on port 18437 using IP-address SANs for the current private addresses.
It also retains the trusted-LAN HTTP listener on port 18436 for existing clients.
The native `get_lan_server_info` result includes the public `localCaPem` and its
SHA-256 `localCaFingerprint` for export and verification; it never includes the
private key. `GET /api/lan-ca.crt` serves the public root certificate. Each
browser device must explicitly trust that root before using microphone capture;
users should compare its fingerprint against native Settings before trusting it.
The root persists across app restarts; the IP leaf is regenerated at startup.
An invalid or incomplete stored root fails LAN startup rather than silently
replacing its identity.

An optional `lan-tls.json` in the app data directory replaces the local TLS
identity with a custom hostname and absolute `certPath` and `keyPath` values
(the hostname has no scheme or port). The complete `MONITTER_LAN_TLS_HOST`,
`MONITTER_LAN_TLS_CERT`, and `MONITTER_LAN_TLS_KEY` environment trio overrides
that file. With a custom identity, HTTP binds loopback only, and the public
local-root endpoint is unavailable. A partial or invalid custom configuration
fails LAN startup. Custom certificate renewal requires a desktop restart. A
browser separately verifies certificate trust, expiry, and hostname. DNS-01 can
issue a publicly trusted custom certificate without exposing the server to the
Internet, but no domain or DNS configuration is needed for the default path.

The server prefers live web assets
in the app data folder's `lan-web` directory, with bundled assets as a fallback.
On macOS, `npm run build` builds and publishes there without rebuilding Rust or restarting
Monitter. `npm run publish:lan` publishes an already completed build. Publication installs
assets before atomically replacing `index.html`, retaining old hashed chunks for open tabs.
Refresh the browser to load the new interface. This updates LAN browsers only; the desktop
webview remains bundled. Backend/API changes still require a matching app rebuild.
Settings → LAN access displays local addresses and a random
per-process six-digit access code (`get_lan_server_info {}` -> `{urls, token, error, accessCodeRequired, localCaPem?, localCaFingerprint?}`;
the `token` string preserves leading zeros). Five incorrect guesses lock API access
for five minutes across all clients; successful polling does not reset the counter. The server
starts with the app and stops with it. A bind failure is shown in Settings.

During trusted-LAN prototyping, `REQUIRE_ACCESS_CODE` is false: the code gate and guess
limiter are bypassed, while private/loopback peer and same-origin checks remain enforced.
The HTTP listener accepts only literal local Host values; the HTTPS listener requires
an advertised IP or custom DNS hostname in Host, with the POST Origin matching
that exact Host. `GET /api/access` returns
`{required:false}` so browsers skip the gate.
Anyone on the trusted LAN has owner-level access, including terminals. The code and limiter
implementation remain available to re-enable later; this is intentionally not Internet-facing.

Browser API calls use `POST /api/invoke {command, args}` with a Bearer access code;
responses are `{ok: true, result}` or `{ok: false, error}`. The browser sign-in gate
stores the code in sessionStorage. Access links place the code in a fragment, which is
removed after loading. Static assets are public; workspace data and commands require
authentication and same-origin requests. The code grants owner-level workspace control,
including terminals; this is distinct from limited visitor sharing. The legacy HTTP
LAN listener is unencrypted. HTTPS encrypts the connection but does not turn the trusted-LAN
owner interface into visitor access. Restarting the app invalidates browser access codes.

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
Both bundled phone shells enter the separately approved owner workspace through
the encrypted relay after initial controller pairing. The Android shell retains
its limited-controller route as an explicit fallback; its bundled HTTPS root is
the owner workspace, while the bundled `/mobile` route handles pairing. The iOS
shell also offers an owner LAN mode. That mode loads the desktop's live
`/` site in WKWebView at an explicitly entered port-18436 address, so its narrow
layout uses the same owner LAN command surface as a phone browser. The address
is saved locally without an access code; LAN authentication, when required,
remains in the web site. Its native QR bridge is accepted only from the separate
loopback `/mobile` origin. The paired controller remains a distinct, limited
mode and uses a stable local loopback origin for its saved pairing state.
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

The network address of either device is not part of its pairing identity: both
connect outbound to the invitation's public WSS relay. Reconnection retains the
room and pinned keys and derives a fresh encrypted session. The relay's join
handshake may negotiate `relay-heartbeat-v1`: a client offers it in `join.capabilities`
and enables it only when `joined.capabilities` advertises it back. Negotiated
clients send the exact frame `{"type":"ping"}` immediately after joining and
15 seconds after each exact `{"type":"pong"}` reply. A missing reply for 10 seconds
closes the local session, rejects outstanding operations, and enters the existing
bounded reconnect loop. These transport acknowledgements never grant device or
owner-workspace approval and do not renew the device's authenticated-access policy.
Legacy relays receive no new heartbeat frames; legacy clients retain their existing
pairing behaviour. The relay expires negotiated sockets after 45 seconds without a
heartbeat, including apparently open stale sockets, before allowing a new socket to
occupy the role. A room ID alone cannot displace a live peer. Cloudflare uses its
hibernation auto-response timestamp for this lease; application payloads remain opaque.

Desktop, mobile bootstrap, and owner-workspace routes coalesce foreground, online,
pageshow, and native `monitter:resume` signals before reconnecting. iOS emits the
native hint for foreground and satisfied network-path changes only into its trusted
bundled paired page. Reconnection never clears saved trust or replays an uncertain
mutation. Physical device handoff testing is separate from local fault-proxy tests.

Once connected, the phone keeps its mounted workspace, readable history, navigation,
and drafts during recovery. Only a small noninteractive spinner indicates automatic
reconnection; background polling does not display transient connection errors.
An attempted network action reports that reconnection is in progress. A send made
while already disconnected retains the draft and is not queued or automatically
replayed. If a connection fails during an already-dispatched send, its existing
unconfirmed-send handling remains in effect. Initial pairing, explicit revocation,
and incompatible desktop versions still show actionable setup information.

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
The initial visitor transcript is bounded. Optional older-message pages are authorized
against the active share both before and after the owner read, filtered to the same
approved task, and projected without owner-local attachment paths or saved system
instructions. Ending or changing a share rejects in-flight pages.
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

## Scheduler

Monitter ships an in-process scheduler that runs as a long-lived background thread
inside the desktop app. It owns a list of `Schedule` rows persisted alongside the
rest of the durable state, computes the next firing instant in the schedule's
resolved timezone, and dispatches on the schedule's `mode`. The thread starts at
app startup and stops when the desktop exits.

### Property set

- **Runs only while the desktop app is running.** If Monitter is closed, crashed,
  or the host is sleeping without an auto-launch, schedules do not fire. There is
  no separate OS-level scheduler, no launchd plist, no cron entry. This is a
  deliberate product choice; users who need schedules to fire while the desktop
  is closed must keep the desktop running.
- **One catch-up fire on startup.** If a schedule was due while the app was off,
  the loop dispatches at most one fire for that schedule on next launch. Multiple
  missed ticks are never replayed; the next scheduled fire proceeds normally.
- **Rust-side decision.** Fires are decided by the scheduler loop reading the
  persisted `Schedule` row. The Tauri and chat-MCP command surfaces are
  configuration only. There is no chat-driven code path that bypasses the loop to
  "force fire now" without going through `run_schedule_now`, which records a run
  and respects overlap and mode semantics.
- **Three modes per schedule.** `persistent_thread` reuses one task per schedule
  and appends the prompt to it on every fire. `new_thread_per_fire` creates a
  fresh task for every fire and leaves it in the user's chat history.
  `throwaway` creates a task, writes a Markdown report under the task folder on
  success, then deletes the task; on failure the task is auto-archived.
- **Overlap handling.** `persistent_thread` schedules can `queue` the next fire
  into the existing task or `skip` it when the previous fire is still running.
  `new_thread_per_fire` and `throwaway` always skip because queuing an ephemeral
  task makes no sense.
- **Auto-pause.** When `maxConsecutiveFailures` is set and the schedule crosses
  that threshold of consecutive failed runs, the schedule is disabled and a
  `missed` system event is appended. `null` disables auto-pause entirely.
- **Frequency is a 5-field cron string.** `m h dom mon dow`, evaluated by
  `croner` against the schedule's resolved timezone. The chat-MCP surface accepts
  friendly preset names (`every_15_minutes`, `daily_9am`, `weekday_mornings`,
  etc.) which the backend translates to cron at write time. Cron is the only
  source of truth on disk. Timezones: `host` for the OS local zone, otherwise an
  IANA name like `Australia/Sydney`.
- **Run log.** Every fire produces one `ScheduleRun` row with `status` of
  `succeeded`, `error`, `skipped`, or `missed`. The log is bounded to the most
  recent 200 runs per schedule and 500 globally; older rows are evicted
  deterministically. Each fire also produces a `RunEvent.kind: 'schedule'`
  activity row visible in the task's diagnostic pane.
- **No live harness requirement.** The scheduler thread does not require a
  harness session to be running. If the harness is not authenticated at the
  scheduled moment, the dispatch records a `ScheduleRun.status: 'error'` with a
  visible message and applies the auto-pause rule.

### Fields

- `Schedule { id, title, agentId, prompt, frequency, tz, mode, overlapPolicy, maxConsecutiveFailures, enabled, lastFireAtMs, consecutiveFailures, createdAt, updatedAt }`
- `ScheduleRun { id, scheduleId, firedAtMs, mode, taskId?, status, finishedAtMs?, summary, error?, skippedOverlap }`
- `ScheduleMode = 'persistent_thread' | 'new_thread_per_fire' | 'throwaway'`
- `OverlapPolicy = 'skip' | 'queue'`
- `ScheduleRunStatus = 'succeeded' | 'error' | 'skipped' | 'missed'`

### Age-based chat archiving

Age-based chat archiving is an opt-in, local desktop maintenance feature. There is no rule by
default, and it does not make model calls. A native Rust scheduler evaluates enabled rules every
15 seconds only while the desktop is running. It creates no OS-level job and does not change any
chat or setting until the user explicitly saves a rule or applies a one-time archive.

`Snapshot.chatArchiveRules` is persisted as `{ projectId, olderThanDays, enabled, createdAt,
updatedAt }[]`; `projectId: null` is the global fallback. MCP inputs always require explicit
`scope`: `global` rejects `project_id`, while `project` requires an existing project UUID.
`olderThanDays` is an integer from 1 through 36,500. There is at most one global rule and one
override per project. A project rule overrides the global rule even if its `enabled` value is false;
deleting that project override restores global inheritance. Deleting the global rule leaves no
global rule. Saving a rule without `enabled` sets it to true.

The authenticated, opted-in collaboration MCP exposes these tools; they are absent from visitor,
LAN mutation, and unauthenticated surfaces:

- `list_chat_archive_rules {}` reads current rules, project IDs/names, and the check interval;
  it is available to read-only tasks.
- `save_chat_archive_rule { scope, project_id?, older_than_days, enabled? }` creates or replaces
  the explicit scope rule. For example, `{ "scope": "global", "older_than_days": 90 }` enables
  a 90-day global rule, while `{ "scope": "project", "project_id": "…", "older_than_days": 30,
  "enabled": false }` creates a disabled project override.
- `delete_chat_archive_rule { scope, project_id? }` deletes that explicit rule. For example,
  deleting `{ "scope": "project", "project_id": "…" }` makes the project inherit the global
  rule again.
- `archive_chats { scope, project_id?, older_than_days, dry_run? }` is a one-time operation that
  ignores recurring rules. It previews by default (`dry_run: true`); `dry_run: false` applies the
  explicit scoped archive. Read-only tasks may list rules and preview only.

Eligibility uses a strict cutoff from the latest task creation/update, message, collaboration,
subagent, and work-plan activity. A day is exactly 24 hours. The scheduler and one-time operation
skip already archived chats, internal Admin chats, running or runtime-resident work, queued or
sending messages, pending approvals, live collaboration or subagents, active work plans, scheduled
persistent-thread chats (including paused schedules), pending throwaway cleanup, and ancestors
of protected tasks. Eligible archive operations preserve chat history and use the normal
unarchive behavior.

One-time replies include `matchedCount`, `archivedCount`, `skippedProtectedCount`,
`skippedByReason`, and up to 100 matching task summaries with `tasksTruncated` explicitly marking
a longer result. The count covers the entire operation, not just the returned summaries. A preview
is advisory; applying rechecks current activity under the state writer lock. Successful archives
write a status event per chat. Native scheduler failures appear in the desktop diagnostic log.

Workspace switching hides native browser children without closing or navigating them.
Process-local tab ownership survives pane remounts, retaining forms, scroll, history
and JavaScript state. Only explicit close/unload destroys the page; app restarts
restore metadata as unloaded because in-memory page state cannot survive exit.
Mounted dialogs (including Usage and Process metrics) suspend native browser
overlays through their exit animation, then reveal the same live pages.
