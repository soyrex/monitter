# Native Neovim panes

Open **New Neovim pane** from the command palette or choose **Neovim** in an
empty pane. This launches the local installed `nvim --embed`, using the user's
Neovim configuration. Neovim owns files, buffers, undo, plugins and editing modes.
Monitter renders its grid, command/search line, completion and messages natively.

`:split` and `:vsplit` create Monitter dock panes backed by windows in the same
Neovim process. Buffers are shared. Monitter owns pane placement and navigation,
including navigation between editors, terminals and chats. Plugin floating
windows belong to the editor overlay rather than becoming workspace splits.

Standard Monitter shortcuts remain available. Settings → Keyboard shortcuts →
**Standard + Vim alternates** enables `Ctrl+W`, followed by `h/j/k/l` or arrows,
across pane types. `Ctrl+W s/v` splits the focused editor (or workspace pane).
The prefix is reserved in ordinary terminal panes when enabled. Neovim retains
its editing meaning for Ctrl+W in insert, command-line and terminal modes.
The former Monitter `Escape :` command emulation is removed from the UI.

Closing an editor invokes Neovim's ordinary window/quit operation, so unsaved
changes remain protected. Use Neovim's explicit `:q!` to discard changes.
Closing/reopening the GUI preserves editor processes in the persistent backend.
Backend shutdown does not provide durable recovery of unsaved buffers.

This first implementation launches local Neovim sessions. It does not attach to
an existing terminal Neovim or launch Neovim over SSH. Floating plugin windows,
completion, mouse input and IME have basic native support; full plugin UI parity,
drag selection and complex nested floating-window placement are not yet verified.

## Private backend protocol

These commands are private GPUI same-user backend IPC. They do not extend the
Tauri, LAN, mobile or shared-web API. IDs are session UUIDs; window and grid IDs
are Neovim integer handles.

| Command | Arguments | Result |
| --- | --- | --- |
| `neovim_available` | none | availability, executable, version, error |
| `open_neovim` | optional absolute local `cwd`, `path`, `cols`, `rows`, test-only `clean` | session descriptor |
| `list_neovim` | none | session descriptors |
| `read_neovim` | `id`, `after` sequence | `seq`, `latestSeq`, `events`, `gap`, `layout`, `windows`, `currentWindow`, `mode`, `exited`, `error` |
| `neovim_input` | `id`, `keys` | accepted byte count |
| `neovim_paste` | `id`, `text` | null |
| `neovim_focus` | `id`, `windowId` | null |
| `neovim_resize` | `id`, `grid`, `cols`, `rows` | null |
| `neovim_command` | `id`, `command` | null |
| `neovim_close_window` | `id`, `windowId`, optional `force` | null |
| `neovim_mouse` | `id`, optional `windowId`, `grid`, `button`, `action`, `modifier`, `row`, `col` | null |

Each replay entry is `{seq, events}`, where events contains an unmodified Neovim
redraw batch. An event groups one or more argument arrays, for example
`["grid_resize", [1, 100, 30], [2, 100, 29]]`. Present only complete `flush`
frames. A replay gap requires model reset; the backend reattaches the UI to
rebuild highlights and grids. Replay and RPC queues have explicit bounds.
Resize commands are queued notifications: their return confirms enqueueing;
the subsequent `grid_resize` event confirms the applied dimensions. This keeps
command-line prompts responsive when Neovim defers resizing.

## Isolated development

Desktop source is in the GPUI checkout; build with `npm run build:gpui`.
`MONITTER_GPUI_PROFILE_DIR` can select an absolute preview profile whose basename
is `com.monitter.gpui.experimental`. Use a separate preview bundle identity and
profile when testing alongside the installed app. Never substitute a Tauri build.

Focused real-Neovim tests:

```sh
node scripts/gpui-toolchain.mjs cargo test --manifest-path scripts/neovim-smoke/Cargo.toml -- --include-ignored
```

The tests launch `nvim --clean`, edit only temporary files and shut down their
own editor processes. Runtime acceptance and any remaining limitations are
recorded in `NEOVIM-IMPLEMENTATION.md`.
