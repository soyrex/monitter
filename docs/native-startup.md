# Native workspace startup

The desktop paints a bounded local presentation of the last saved workspace
before waiting for the persistent backend. There is no splash or fade gate.
`workspace-presentation.json` is disposable client data in the native profile,
written atomically with mode 0600 and limited to 2 MiB. It contains layout,
appearance, shell identities and short text previews; no attachment bodies,
drafts, credentials, approval payloads or journal cursor. A missing, invalid or
incompatible cache is ignored and the ordinary shell opens while state loads.

The session-lock store must authorize disclosure before cached text is shown.
Visible cached chats use retained native panes and an initially disconnected
service facade. Their composer accepts local editing; backend mutations and
workspace/settings actions remain blocked until authoritative restoration and
draft hydration finish. Quit, lock and recovery remain available. `NativeState`
always starts empty and only accepts authoritative responses. A successful
connection binds the existing facade without replacing the cached editor.

Saved drafts are merged with text entered during startup without truncation.
IME composition and pending attachment edits defer hydration; command readiness
and durable close remain gated until that merge completes. Cached presentation
data never authorizes a backend action or supplies a journal cursor.

Connection and subscription setup run off the GUI thread. The native-only
`read_native_ui_changes` projection sends task metadata and only requested
message tails; visible chats hydrate through revision-checked page requests.
Only a successful page proves that a chat has no more history. Full resets
invalidate page coverage; ordinary deltas retain previously accepted pages.
An older running backend is detected once per connection and retains its legacy
API until it is restarted normally. Updating the GUI never terminates it.

Workspace restoration settles sidebar widths/visibility and selected detail
tabs before painting. It cancels outgoing automatic sizing and restores saved
proportions. Explicit subsequent toggles, tab selections and sizing actions
retain their animations.

Logs with prefix `[native-startup]` record milestones in milliseconds since Rust
`main`. These exclude operating-system launch/dynamic-linker time. Activation is
an activation request; a cached-editor callback is not proof of draft hydration
or command readiness. Separate milestones record `startup-drafts-hydrated`,
`workspace-actions-ready`, and an active, visible
`workspace-actions-paint-observed`. Paint observations do not measure physical
display scanout, and do not imply that browser pages or older chat history have
loaded. Timing metadata is written in the background to `startup-timing.json`
with a process ID and main-entry wall timestamp for external launch probes.

Workspace switches retain panes and prepared transcript state within the same
native window. Preference saves are generation checked and do not gate selecting
an already restored workspace. With `MONITTER_WORKSPACE_TIMING=1`, logs separate
prepared and unprepared-at-request switches and record handler-to-shell and
handler-to-content paint. A prepared switch requires every visible chat to have
accepted history and a current prepared viewport. Failed, superseded and
incomplete switches must remain visible in reports alongside completed timings.

Closing chat, browser or terminal panes releases their GUI views while backend
tasks and terminal sessions continue. Hidden workspace panes remain retained;
transcript painting is deferred and terminal polling is paused. Neovim's close
action closes its editor window; after confirmation its GUI pane is released.
A retained Neovim session with no GUI panes now parks its client poller until
a pane is recreated.
