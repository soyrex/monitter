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
- `rename_task { id: string, title: string }` -> Snapshot
- `set_task_archived { taskId: string, archived: boolean }` -> Snapshot (reject running; preserve all history)
- `get_task_goal { taskId: string }` -> Goal | null (read-only Codex app-server lookup; version-dependent)
- `delete_task { id: string }` -> Snapshot (archived only; reject running; preserve native CLI history)
- `send_message { taskId: string, text: string, attachmentIds?: string[] }` -> Snapshot (starts asynchronously)
- `cancel_task { taskId: string }` -> Snapshot
- `save_settings { settings: Settings }` -> Snapshot
- `save_channel { channel: Channel }` -> Snapshot (empty id creates; preserve existing messages)
- `send_channel_message { channelId: string, text: string, agentIds: string[], attachmentIds?: string[] }` -> Snapshot
  Explicit selected/mentioned recipients only. Each recipient has a dedicated task under channelId;
  initial/follow-up prompt includes recent channel context. Final replies mirror into channel messages.
- `resume_task { taskId: string }` -> Snapshot (continue the existing native session asynchronously in this chat)

Event `monitter:changed` payload `{ taskId?: string }` tells UI to reload snapshot (debounce <=150ms).
The backend is authoritative; listen before initial snapshot. Errors reject with a readable string.
Frontend may show a labelled browser design preview when Tauri isn't available, but never simulate an agent reply.

## Agent identity

`Agent.avatar` is a nullable embedded PNG/JPEG/WebP Base64 data URL. Missing avatars migrate to null.
The frontend accepts files up to 2 MiB; the backend rejects remote URLs, SVG, malformed Base64 and
serialized values above 3 MiB. Images stay in the app's private local state; selecting or removing
an avatar does not alter the agent's native session. The sidebar and run detail fall back to initials.

An existing chat header contains its subject, Resume and the task overflow menu, plus Stop while
running. Folder, harness, model, host, project and permissions appear at the top of run detail with
Agent identity. The lowercase monitter menu contains Preferences, Hosts and Agent directory; no separate host row
is shown in the sidebar.

## Appearance and conversation preferences

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
first message is sent. Draft fields survive navigation within this window; drafts are not durable
across app restart. Closed draft tabs remain discoverable through Cmd-K until the window closes. A failed send after task creation reuses that task on retry. A delayed send
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
Claude uses `--print --output-format stream-json --verbose` and `--resume`; OpenCode uses
`run --format json --thinking` and `--session`. Hermes runs the installed TUI gateway with a Python
bridge: session.create/resume, prompt.submit, durable stored_session_id, normalized JSONL events,
explicit denial of interactive requests, and owned child cleanup. Never silently substitute a harness.
Host.claudePath defaults to an empty string for older stored host snapshots. Task.archived defaults
false; archiving preserves messages/events/native IDs and hides the chat from ordinary navigation.
Cmd-K explicitly restores archived chats. Channel sends start a new task instead of reusing an
archived one. Permanent deletion is available only after archiving. The confirmation preserves CLI history by default; an optional switch requests verified native session file cleanup as described below.

Persist configuration and transcript in the Tauri app data directory, private permissions, atomic writes.
Recover formerly running tasks as interrupted after restart. One active turn per task and provider/host/native-session key.
Store task host/cwd/provider/model/sandbox as a snapshot when created; agent edits affect new tasks.
Codex defaults to read-only with explicit workspace-write selection. Other providers require
`harness-configured`; do not describe their host permission rules as an OS sandbox. Never use a bypass-all-permissions flag.
Noninteractive exec cannot answer approval prompts: show actual tool failures and allow changing policy
for a new task; do not present nonfunctional Approve buttons. No auto-resubmission after errors.

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
