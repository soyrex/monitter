# Monitter desktop contract

Tauri 2 / Svelte 5 / TypeScript / Rust. macOS Apple silicon first.
Canonical TS data shapes: `src/lib/types.ts`. Rust serde fields use camelCase.
All timestamps are Unix milliseconds. IDs are UUID strings. Optional task links use null.
No fake conversations, progress, token counts, host connections or model replies in the shipped app.

## Commands (Tauri invoke names and JSON argument keys)

- `get_snapshot {}` -> Snapshot
- `save_host { host: Host }` -> Snapshot (empty id creates)
- `delete_host { id: string }` -> Snapshot (reject referenced/default local host)
- `probe_host { host: Host }` -> ProbeResult (unsaved settings allowed; versions keyed provider)
- `save_agent { agent: Agent }` -> Snapshot (empty id creates)
- `delete_agent { id: string }` -> Snapshot (reject if tasks exist)
- `save_project { project: Project }` -> Snapshot (empty id creates)
- `delete_project { id: string }` -> Snapshot (unassign chats; preserve their history and runtime)
- `set_task_project { taskId: string, projectId: string | null }` -> Snapshot
- `create_task { input: CreateTaskInput }` -> Task
- `get_model_catalog { target: { taskId?: string, agentId?: string, projectId?: string | null } }` -> ModelCatalog
- `set_task_model_settings { taskId: string, settings: ModelSettings }` -> Snapshot
- `set_task_sandbox { taskId: string, sandbox: Sandbox }` -> Snapshot (idle, unarchived tasks only)
- `rename_task { id: string, title: string }` -> Snapshot
- `autoname { target: { taskId?: string, channelId?: string, terminalId?: string, content?: string } }` -> Snapshot
- `set_task_archived { taskId: string, archived: boolean }` -> Snapshot (reject running; preserve all history)
- `get_task_goal { taskId: string }` -> Goal | null (read-only Codex app-server lookup; version-dependent)
- `delete_task { id: string }` -> Snapshot (archived only; reject running; preserve native CLI history)
- `send_message { taskId: string, text: string, attachmentIds?: string[] }` -> Snapshot (starts asynchronously)
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
- `resume_task { taskId: string }` -> Snapshot (continue the existing native session asynchronously in this chat)

Event `monitter:changed` payload `{ taskId?: string }` tells UI to reload snapshot (debounce <=150ms).
The backend is authoritative; listen before initial snapshot. Errors reject with a readable string.
Frontend may show a labelled browser design preview when Tauri isn't available, but never simulate an agent reply.

## Busy messages

`Settings.busyMessageMode` defaults to `queue`. `Snapshot.queuedMessages` persists FIFO records per
task, including the original text, attachment IDs, optional channel, status and error. A busy direct
chat or selected busy channel recipient queues its follow-up; idle recipients still start immediately,
and a channel user message is recorded once at submission rather than again when a recipient drains.
Current CLI adapters do not support live steering, so `steer` safely queues and records a visible
fallback event. The next queued item starts only after its owned native run releases. A restart changes
an uncertain `sending` item to `error` and never replays it automatically. Cancelling, kicking,
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

Base sizes `interfaceFontSize`, `chatFontSize`, and `terminalFontSize` are integer pixels (8–32), defaulting to 14, 13, and 14 respectively. Interface typography retains its relative hierarchy; native interface zoom scales all three exactly once.

Settings include optional `interfaceFont`, `chatFont`, and `terminalFont` family names. Empty or missing values use IBM Plex Sans for interface/chat and IBM Plex Mono for terminals. Installed custom family names are supported with fallback fonts; changes apply to existing terminals without restarting sessions.

Settings include `accent`, `theme`, `interfaceScale` (integer percent, 80–200, default 125),
`showToolActivity` and `showReasoningSummaries` (default true), `sendWithEnter` (default false),
and `sidebarView` (`standard`, `activity`, or `projects`; default `standard`).
Missing new fields receive these defaults when older saved workspaces load. Native WebView zoom
scales the whole interface; window controls keep their native size and reserved header space.
Cmd/Ctrl+plus (including Cmd+=) and Cmd/Ctrl+minus adjust the saved setting by five percentage
points within 80–200%. Rapid shortcuts accumulate while writes are pending; they do not trigger
an independent browser zoom.
Activity toggles filter both inline blocks and run detail without deleting captured events.
Only reasoning summaries emitted by the harness can be rendered; missing reasoning is not fabricated.
Enter-to-send applies to tasks and channels. Shift+Enter always inserts a newline; IME composition
never submits a message. With Enter-to-send off, Cmd/Ctrl+Enter submits instead.

Open task tabs live in the window header. Closing a tab preserves the task and its draft; reopening
from the sidebar restores it. Tab selection is window UI state, separate from persisted task history.

New chat opens a local draft tab with message, agent and project choices, starter suggestions,
and optional title/native session attachment. No task or harness process is created before the
first message is sent. Draft fields, unsent text and attachment references are saved with the desktop
workspace and survive navigation and restart. Closed draft tabs remain discoverable through Cmd-K.
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

The saved sidebar view offers Standard (agent groups), Activity (running first, then most recent),
and Projects (collapsible project folders plus unassigned chats). Channels remain available in each
view. Archived chats stay out of these ordinary lists and remain discoverable through Cmd-K.
Projects participate in the switcher; controls include new project and sidebar view selection.

## Runtime and persistence

Orbit-inspired native CLI processes: Codex `exec --json` and `exec resume --json <id>`, prompt via stdin.
Resume is explicit by native session ID. Users may attach an existing idle native session by ID; old
transcript import and live takeover of another Desktop/TUI process are not implied.
Codex event stream is JSONL item/turn events; do not promise token deltas unavailable in exec mode.
Use real CLI account/config; do not copy auth or change global config. Resolve local CLI paths even
when launched by Finder with limited PATH (user local bin, Homebrew, standard dirs).
Claude uses one resident `--print --input-format stream-json --output-format stream-json --verbose`
process per local desktop chat. It receives subsequent user turns and permission decisions over the
same JSON-lines stdin, retaining its native context; `--resume` is only a controlled process-recovery
or explicit handoff bootstrap, never the normal next-message path. OpenCode uses
`run --format json --thinking --dir <folder>` and `--session`. Before resuming OpenCode,
a bounded read-only `export <session>` lookup verifies the native ID and restores the original
session folder into the task snapshot, before collaboration setup. This prevents inherited PWD
or a mismatched attachment folder from silently losing the CLI event stream. Export contents
are not imported into the chat or logged. Hermes runs the installed TUI gateway with a Python
bridge: session.create/resume, prompt.submit, durable stored_session_id, normalized JSONL events,
explicit denial of interactive requests, and owned child cleanup. Never silently substitute a harness.
Host.claudePath defaults to an empty string for older stored host snapshots. Task.archived defaults
false; archiving preserves messages/events/native IDs and hides the chat from ordinary navigation.
Cmd-K explicitly restores archived chats. Channel sends start a new task instead of reusing an
archived one. Permanent deletion is available only after archiving. The confirmation preserves CLI history by default; an optional switch requests verified native session file cleanup as described below.

Persist configuration and transcript in the Tauri app data directory, private permissions, atomic writes.
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
explicit one-time approve or deny decision. Pending requests remain prominent in the owning chat and
their resolved history is retained. A stopped task, expired response channel, restart or unsupported
provider transport resolves a pending request safely without authorizing work. Never render an approval
denial as a successful `Tool result`, and never render an enabled approval control unless that live
provider run can receive the decision.

Current interactive approval transports are Hermes' local full-duplex gateway and Claude's local
stream-json host protocol. A Claude `can_use_tool` request creates one durable desktop approval and the
chosen one-time allow or deny is returned to that exact control request ID with the provider's original
tool input preserved. SSH Hermes and SSH Claude remain safe-deny/unsupported until their remote
supervisor supports the same persistent full-duplex channel. Codex `exec` and OpenCode `run` still need
their dedicated app-server/ACP adapters before an approval button is shown. No auto-resubmission after
errors.

SSH host port 0 means use the existing SSH config/default (omit `-p`). An empty user/identity path
also preserves the SSH config. Expand remote `~/` relative to the remote home, never the local home.
SSH uses the system client, keys/agent/config, BatchMode, bounded connection timeout and normal host-key
verification. Quote each remote argument using POSIX single quote escaping; prompts always stdin.
Do not disable StrictHostKeyChecking or expose a public network listener. Killing a local process must not
kill unrelated agents. Cancellation and app quit must clean up owned children as far as transport permits.
No purchase or paid provisioning; existing authorized Codex account usage only.

## Activity and navigation

The document/root never scrolls. Each pane has a bounded independent overflow; short/high-scale
windows also bound headers, composer and activity panels. Cmd-K is the workspace switcher and
Cmd-P is the controls palette. Both preserve drafts, contain keyboard focus and support filtering,
arrow navigation, Enter and Escape. Settings use native checkbox inputs styled/announced as switches.

Activating a chat or channel opens its message pane at the latest message. Incoming content follows
the bottom while the reader is already there; scrolling up pauses that behavior and shows a
keyboard-accessible down-arrow button to jump back. Successful sends reveal the submitted message
in the same active conversation. A delayed send acknowledgement cannot scroll a different chat.
Content/pane resizing preserves bottom-following, including late image/font loads and tool expansion.

RunEvent.kind additionally accepts `computer`, `goal`, and `log`. Computer detail contains
`{id,phase:"started"|"completed",tool,summary}`. Only start records in the current user turn of a
running owned task activate the compact panel. Matching completion or terminal state clears it;
historical tool records cannot reactivate control. Stop cancels the task's owned process group.

`get_task_goal` performs only the Codex app-server initialize/initialized/thread/goal/get handshake,
with bounded cleanup. It neither resumes nor changes a thread. A null result hides the panel; API
errors remain inspectable in run detail. Objective/status/token budget/usage come from the harness.
Hermes goal notifications have free-form text and appear as the last reported update without
fabricated objective/status/token metrics. OpenCode plans are not presented as native goals.

## Slash menu

Typing `/` opens a filtered, keyboard-accessible menu labelled Monitter commands. These are app
actions: `/new` opens an independent draft, `/settings` opens preferences, `/project` selects the
chat's project, `/stop` cancels a running task, `/resume` continues the saved native session in this chat, and
`/goal` reads the available Codex goal. Task-specific actions only appear in applicable contexts.
`/autoname` names the current chat or channel from its recent messages. Controls → Auto-name current pane also names an active terminal from a bounded recent-output buffer; it is never written to that shell. Naming uses the configured default Codex agent (first Codex agent, then first agent), makes an ephemeral read-only title run with user config/rules ignored and no persisted task or session, and preserves channel history/membership and terminal session identity.
Selection supports arrows, Enter, Escape and clicking. IME composition does not select an action.

The current CLI transports do not expose a shared native slash-command catalog. Unknown commands
and command arguments such as `/goal objective` produce a visible explanation without sending them
as ordinary model text. The Send button and keyboard use the same dispatch rule. `//` explicitly
escapes a literal leading slash; paths such as `/path/to/file` remain ordinary messages. Full native
command discovery/execution needs separate harness adapters and is not implied by this menu.

## Cross-agent collaboration

Agents publish `expertise`, `responsibilities`, and `skills` string arrays alongside their description.
`collaborationEnabled` defaults to true; disabling it removes the agent from harness discovery and
prevents new routed deliveries. Agent profile and instructions are captured for a newly created chat;
editing a profile changes directory discovery immediately and the instructions of future chats.
Profiles contain up to 40 entries per field, each at most 512 bytes. Full instructions and host
credentials are never included in the directory.

`Snapshot.collaborations` defaults to an empty array. Each record has an ID, kind (`message` or
`delegation`), originating agent/task, recipient agent/task, original text, caller-scoped request ID,
status, result/error and timestamps. `Message.senderAgentId` and `collaborationId` are optional and
identify real peer messages/results. These records and their task links are persisted atomically.
The Agent directory is available through the monitter menu and Cmd-P; profile editing and task run
detail expose capabilities, collaboration availability, linked chats, delivery state and results.

A process-local broker binds only to `127.0.0.1` on a random port. Every running task receives its own
in-memory bearer grant; the broker derives caller identity from that grant, never from tool arguments.
Grants are revoked on run completion. A bundled Python stdio MCP helper exposes a narrow tool set:

- `list_agents`: search configured, published agents by profile.
- `delegate_task`: create a child chat, inherit its parent's project, apply the recipient's saved
  host/folder/model/permissions, and durably queue the supplied brief.
- `send_message`: send a brief to a peer, optionally using a directly connected collaboration chat.
- `get_task_result`, `wait_for_task`: read the actual result and incoming peer messages.
- `list_messages`: deliver queued messages to this active turn and return its durable inbox.
- `cancel_delegation`: cancel an active delegation owned by this task.

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

SSH uses an owned reverse forward bound to remote loopback and a private temporary helper. Task
credentials travel over SSH stdin into the remote environment; they never enter command arguments,
persisted state, command previews or logs. No public listener or persistent remote service is installed.
The broker rejects browser-origin requests, invalid/revoked grants, oversized input and unknown tools.

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

The selected workspace remains visible independently of the selected chat. Agent/project sidebar
badges and the global approval entry surface pending approvals even in hidden workspaces. Opening
an approval navigates to the shared owning chat. Global search can cross workspaces; tab cycling and
split/move operations act only inside the current workspace.

UI persistence uses `monitter.workspaces.v2` with an active workspace key and a collection of the
existing pane snapshots. Migration retains `monitter.workspace.v1` and imports its complete layout
into All. A failed or unsupported saved-workspace read must not overwrite the stored data. Hidden
workspace terminal references are preserved too; reload reuses live shells, while app restart can
open fresh shells at their saved host/folder, as described in Terminal tabs. Deleted/reassigned chats
are reconciled against current agent/project membership without deleting their history or drafts.

The main sidebar can collapse to agent avatars with chat popovers. Its single view menu selects
Standard, Activity or Projects; nonstandard chat rows include the owning agent's avatar. Chat archive
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

## Composer models and pane appearance

The composer shows its current model beside Send, with the attachment button at bottom left.
Codex catalogs come from that task's saved host (local or SSH) through read-only app-server
`config/read` and paginated `model/list`, cached by host/provider/working folder for 60 seconds. The menu exposes
only advertised models, reasoning levels and Fast capability. Other harnesses currently report that
catalog discovery is unavailable; they keep their configured models. Catalog failures remain visible.

`ModelSettings` contains `model`, nullable `reasoningEffort`, and nullable `fastMode`. New drafts keep
these choices locally until their first send. Existing idle chats retain native session ID, host,
folder and history when changing model. Running or archived chats reject changes. Codex receives
invocation-only model/effort/service-tier overrides; Fast uses the advertised `priority` tier, explicit
off uses `default` only for models advertising that capability, and resetting clears overrides.
Models without advertised Fast support send no service-tier override. User CLI configuration and authentication are
never edited. The Resume icon appears on interrupted/error native sessions; the backend also rejects
Resume while the previous owned process is still active. Ordinary completed chats continue on send.

Every pane uses the same tab-bar height, including native macOS zoom compensation. Split separators
paint a one-pixel line with a wider invisible drag target. Agent messages show the sending agent's
small avatar. `Settings.dimInactivePanes` defaults to true and `inactivePaneOpacity` to 0.6; Preferences
provides a toggle and 10–90% slider, with validation requiring a finite value from 0.1 to 0.9.
Pointer or keyboard focus immediately marks the receiving pane active; its opacity is always 1.


Dimmed panes also desaturate completely; focusing a pane or disabling dimming restores its colour.
Task and channel title backgrounds use 80% opacity and a 14px backdrop blur, with opaque text.
Titles remain sticky inside their own message scroller so actual conversation content passes behind
them. The combined title/activity area has a bounded independent overflow for small panes.

Consecutive tool entries of the same tool or connector family share one muted, borderless row
with extra spacing below. Connector methods such as `gmail.search_emails` and `gmail.read_email`
share a family; the visible description and timestamp follow the latest entry. Clicking opens a
bounded, scrollable top-layer popup containing the full list and each entry's original details.
Escape or clicking outside dismisses it. Matching streamed entries extend an open popup.
Grouping is presentation-only: raw events, details, timestamps and settings remain unchanged.
Messages, reasoning and different services separate groups. Counts say “entries” because harnesses
can emit multiple lifecycle updates for a single call. Individual entry details can collapse.

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
Categories are Agents, Agent directory, Appearance, Typography, Permissions & Behaviour, and Conversation. Existing controls
save automatically. Permissions remain per-agent/task; the page does not introduce a global bypass.
Settings edits and command-palette preference changes use a shared queue that merges each patch
into the latest saved settings. Failed saves remain visible; switching tabs preserves the settings
page's local state and the chat's unsent draft.

Conversation display: `Settings.tintUserMessages` defaults to false for existing and new installs. When enabled, user bubbles in direct chats and channels use a subtle accent tint; message content and agent replies are unchanged.


`autoname { target: { taskId?: string, channelId?: string, terminalId?: string, content?: string } }` returns Snapshot. Exactly one target is required; terminal text comes from a bounded, control-sequence-scrubbed client buffer. Chat/channel context uses the last 12 messages, bounded to 12,000 characters. The first configured Codex agent supplies the naming host/folder. An advertised Spark/Luna/Mini model is preferred, otherwise the Codex harness default is used. The ephemeral read-only invocation ignores user config/rules and does not resume the working session. Codex built-in read tools remain available; this is not a guaranteed tool-free API. Unsupported CLI isolation flags fail visibly rather than falling back to the working session. The title subprocess times out after 45 seconds, with input/output handled concurrently and the SSH control pipe held open until completion. Titles are bounded to 60 characters. Terminal names update the existing session and are refreshed into the tab runtime. Use /autoname in chat or find /autoname in the Controls palette (Cmd-P on macOS) for a terminal; the shell never receives the command.


### Pane controls and window chrome

The main sidebar toggle is the first control in the main pane tab bar. The sidebar brand and
Standard/Activity/Projects icons share its top row. Native macOS window-control clearance is
removed in fullscreen and restored on exit, using the native window fullscreen state.
Right-sidebar toggles live beside expansion in chat/channel headers only, because those are
the pane types with run detail or channel-member sidebars. Compact sidebar blades retain
their own close control. Expansion controls remain reachable when their tab bar is hidden.


`Settings.compressToolCalls` defaults to false. When enabled, consecutive visible tool
events across tool families render as one count (for example `9 tool calls · 4.6s`).
Messages and displayed reasoning remain boundaries. The duration is the recorded
first-to-last event span, not summed tool execution time. Clicking expands an inline
scrollable box containing every original event; disabling compression restores family
grouping. Stored events remain unchanged.

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

## Mobile controller

`docs/MOBILE-CONTROLLER.md` describes the paired mobile clients and relay.
Remote control is opt-in. The versioned controller allowlist dispatches only
approved encrypted sessions through the existing desktop bridge. Desktop and
mobile navigation stay independent. Pairing uses ephemeral keys, QR invitations
or one-use nine-digit rendezvous codes, and explicit desktop approval after
comparing verification numbers. Session closure blocks further requests;
already accepted desktop work continues. Mutation receipts are session-local,
so disconnects require new pairing and uncertain sends are never auto-retried.

## Shared operators

Workspace sharing is a separate, opt-in capability built on the same ephemeral,
encrypted one-use pairing transport. The desktop owner enters their display name,
shares a one-use link or quoted nine-digit code, compares the verification code,
and explicitly approves the visitor's declared display name. Pairing by itself
grants no workspace access.

After approval, the owner selects individual chats and/or projects. The visitor
can view and send messages only within that current selection; they cannot stop
or resume agents, create chats, access terminals, inspect hosts, folders,
attachments, agent instructions, activity, approval details, queues or other workspace records.
Visitor snapshots use an explicit field projection; new desktop snapshot fields
must be reviewed before they are exposed to visitors. Harness approvals remain
with the desktop owner, even for a shared chat.
Ending sharing revokes the in-memory session immediately. A reconnect needs a
fresh link and approval.

Shared user messages are stored and sent to the model with visible attribution:
`@(Alex): message`. The execution prompt also names the two operators, such as
`Alex (primary user)` and `Luke (visitor)`, so attribution is model context
rather than an implicit authority change. The UI renders the operator name and
avatar separately from the message text.
