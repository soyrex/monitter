# Native workspace startup

The desktop paints a bounded local presentation of the last saved workspace
before waiting for the persistent backend. There is no splash or fade gate.
`workspace-presentation.json` is disposable client data in the native profile,
written atomically with mode 0600 and limited to 2 MiB. It contains layout,
appearance, shell identities and short text previews; no attachment bodies,
drafts, credentials, approval payloads or journal cursor. A missing, invalid or
incompatible cache is ignored and the ordinary shell opens while state loads.

The cached dock has no backend services. Its input is inert, and native
workspace/settings actions are blocked until live restoration finishes. Quit,
lock and recovery remain available. `NativeState` always starts empty and only
accepts authoritative responses. Existing draft merge, session liveness and
workspace validation run before the cached dock is replaced by live panes.

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

Logs with prefix `[native-startup]` record the first workspace paint (cached or
ordinary) and first live workspace paint, in milliseconds since Rust `main`.
These exclude operating-system launch/dynamic-linker time. They are measured
stages, not an assertion that all browser pages or older chat history are loaded.
After the live frame, timing metadata is also written in the background to
`startup-timing.json` in the native profile for launches without a terminal.

Closing chat, browser or terminal panes releases their GUI views while backend
tasks and terminal sessions continue. Hidden workspace panes remain retained;
transcript painting is deferred and terminal polling is paused. Neovim's close
action closes its editor window; after confirmation its GUI pane is released.
A retained Neovim session with no GUI panes now parks its client poller until
a pane is recreated.
