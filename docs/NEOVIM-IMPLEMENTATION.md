# Native Neovim panes

Branch: `feature/native-neovim-panes-20261013`.
Worktree: `/Users/alex/code/monitter-neovim-panes`.
Base: `fac0971` from the GPUI desktop branch. Existing uncommitted changes in
`/Users/alex/code/monitter-gpui` are deliberately outside this branch.

## Goal and ownership

Real local Neovim runs in the persistent native backend. Its RPC UI protocol
feeds a retained GPUI model. Each regular Neovim window becomes a native
Monitter pane; all windows in a session share buffers, mappings and undo state.
Monitter owns physical pane placement and keyboard focus. The adapter forwards
focus and size changes to the corresponding Neovim window/grid. Neovim split
events create adjacent native panes. Floating plugin windows stay overlays.

Neovim normal-mode window chords are alternate Monitter navigation shortcuts
when the Vim shortcut option is enabled. Standard workspace shortcuts remain
available. Insert, command-line and terminal modes retain their editing keys.
The old workspace `:` command emulation is superseded by Neovim's command line.

Process and filesystem operations stay on the backend, behind private same-user
desktop IPC. No Neovim command is added to mobile/LAN permissions. Closing the
GUI must preserve backend sessions. Explicit editor closes go through Neovim
so it can reject losing modified buffers.

## Implementation plan

1. Luna: inspect existing lifecycle/build paths, implement alternate shortcuts.
2. Sol: backend MessagePack RPC, process lifecycle, private command API and real
   Neovim integration tests.
3. MiniMax: pure bounded redraw/grid/command-line model and protocol tests.
4. OpenCode: retained GPUI session/view, rendering and native input.
5. Parent: native dock integration, creation/restore, build, isolated runtime
   validation and corrections.

## Acceptance evidence required

- Discover the actual local Neovim executable and report startup failures.
- Open a local file, edit, save and verify its bytes.
- Display real `:` and `/` command-line state in native chrome.
- Create horizontal, vertical and nested splits; each gets a native pane.
- Navigate between editor panes and a non-editor pane using Monitter actions.
- Resize and refocus panes without cross-window input or repeated focus jumps.
- Reject closing the final modified buffer; permit a deliberate saved close.
- Reconnect the GUI to a surviving backend session.
- Build with the GPUI toolchain and validate in a separate preview profile.

The installed `/Applications/Monitter.app` and live profile are outside this
implementation's launch/install scope. Build cache reuse is permitted only
when no other native build is running; no dependencies are installed merely
to delegate work.

## Validation record (2026-10-13)

- Neovim v0.12.3 from `/opt/homebrew/bin/nvim`.
- Focused smoke harness: 21 passing tests, including actual Neovim file
  editing/saving, shared-window splits, guarded modified-buffer close, real
  command-line redraw decoding, and responsive reads/resizing during a modal
  prompt. Model regressions cover batched protocol arguments and grid shrinking.
- GPUI key-context tests: 2 passing tests. Optional workspace chords apply to
  normal editor, chat, terminal and empty panes, and yield to Neovim insert,
  replace, command-line and terminal editing modes.
- An earlier isolated GUI was closed and reopened against the same backend;
  its Neovim session UUID and unsaved `hello` buffer survived. That preview is
  retained because Alex interacted with its buffer.
- Native runtime verification used the separate
  `artifacts/neovim-verification/com.monitter.gpui.experimental` profile and
  separately identified preview bundles. The backend was started explicitly
  for this fresh profile; subsequent GUI restarts reconnected to it.
- `:vsplit`, then `:split`, produced a native row containing a nested column
  with three editor panes. All three rendered the shared file after shrinking
  to their dock bounds, without blank grids or resize errors.
- `Ctrl-W h/l/j` moved between native editors and a real terminal. A terminal
  marker was visibly typed into the terminal and cleared without execution.
- Distinct-buffer testing caught and corrected stale focus: focus forwarding
  now uses native focus events and generation-matched RPC acknowledgement;
  flushed cursor events take precedence over asynchronous metadata. After a
  left/right/left round trip, right-pane text stayed in its separate buffer,
  while both left-pane markers were saved only to `native-edit.txt`.
- Native `:` and `/` input rendered in the focused pane. Closing a final
  modified buffer via Cmd-W displayed E37 and retained its text. Saving it to
  `guard-check.txt`, checking the file bytes, and closing again succeeded.

Rendered evidence is retained under ignored `artifacts/gpui/visual/`:
`gpui-frame-1791889133108588000.png` (nested splits),
`gpui-frame-1791889157451148000.png` (native command line),
`gpui-frame-1791889236306767000.png` (native search),
`gpui-frame-1791889716459581000.png` (distinct-buffer focus), and
`gpui-frame-1791889778254735000.png` (modified final-buffer protection).

Final build: `artifacts/gpui/debug/build-1791889963294/Monitter GPUI.app`;
`npm run build:gpui -- --visual-export` passed. The final preview also verified
the concise unsaved-change message and immediate pane navigation after a saved
editor closes (`gpui-frame-1791890009355577000.png` and
`gpui-frame-1791890036052993000.png`). Installed application and earlier preview
sessions were not replaced or stopped. This is focused validation, not a claim
that the unrelated full repository test suite is green.

The user configuration remains enabled for normal sessions; tests use
`nvim --clean`. This is a local desktop implementation, not mobile/SSH editor
parity. Plugin floating windows, completion, mouse and IME are initial support;
complex plugin layouts, mouse drag selection, and durable recovery after
backend shutdown remain outside the verified behavior.
