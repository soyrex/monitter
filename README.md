# Monitter

Your agents, together. A macOS workspace for persistent agents and their local or SSH harness sessions.
Built with Tauri 2, Rust, Svelte 5 and TypeScript from Alex's supplied interface designs.

## Development

Requires Node/npm, Rust and Xcode command-line tools. Install dependencies with `npm ci`, then run
`npm run tauri dev`. The development frontend uses loopback port 18420. `npm run dev` is a browser
design preview; agent execution requires the native app.

An installed build can use that same hot-reloading frontend without launching a second native
backend. Run `npm run dev` in the worktree you are editing, then choose **Monitter → Load
Hot-Reload UI** in the app menu. The app verifies Monitter's marker on exact loopback port 18420
before navigating its existing window, so the same native state and running sessions stay attached.
Use the in-window **Use packaged UI** control (or the matching app-menu item) to switch back. If the
Vite server disappears, the window returns to the packaged interface after three failed checks.
Only frontend changes hot reload; Rust/backend changes still require rebuilding and reinstalling.
Because the Vite port is fixed, run one Monitter `npm run dev` server at a time.

```sh
npm run check
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
npm run build:mac
npm run install:mac
```

For a first local build using the development Rust profile, run `npm run build:mac:local`, followed
by `npm run install:mac -- --debug`. This packages the production frontend and the same native
execution service, while retaining Rust debug symbols. The default `build:mac` uses release optimization.

Browser interaction checks run with `npm run dev` in one terminal and `npm run test:ui` in another.
They use a test-only transport and do not invoke an agent. Real harness checks are explicit:
`npm run test:native -- local` or `npm run test:native -- mira` after building the `monitter-smoke`
binary with Cargo. These use the existing authenticated CLI and save evidence in `verification/`.

The macOS package is locally ad-hoc signed for this Mac. Distribution would require Developer ID
signing and notarization. The installer verifies the bundle signature, preserves an existing app,
and never disables Gatekeeper.

## How it works

An agent has a name, instructions, harness, model, host, working folder and optional local avatar image. Each task keeps its own
native session ID and a snapshot of the host/folder/model/permissions chosen when it was created.
Codex runs through the actual installed CLI and uses that host's existing authentication. Monitter
does not call a replacement model API or copy credentials between hosts.

New chat opens a draft tab with a message box, suggestions, and agent/project choices. The harness
starts when you send the first message. You can switch between draft tabs without losing text;
these unsent drafts stay in the current window until you quit.

Projects group chats independently of agents, with optional working folders for each local or SSH
host. Multiple agents can share a project. Its folder applies to new chats; moving an existing chat
preserves its original session and folder. The sidebar switches between Standard agent groups,
Activity ordered by running/recent chats, and collapsible Projects folders.

Appearance includes a saved 80–200% scale slider (125% default), Cmd+plus/minus shortcuts in 5% steps,
theme and accent, independent tool-activity
and reasoning-summary toggles, and Enter-to-send versus Cmd+Enter-to-send. Tool events and available
reasoning summaries appear as collapsible blocks in the conversation. Open task tabs occupy the
integrated macOS window header; closing a tab keeps the task and its draft. Each chat has a compact
subject header, with execution details and agent identity in Run Detail. The monitter dropdown
contains Preferences and Hosts. The window itself never
scrolls: sidebar, messages, run detail, dialogs and long tool output have bounded overflow.
Chats and channels open at the latest message. Scrolling up pauses automatic following and reveals
a down arrow to jump back; sending a message returns that conversation to the bottom.
Cmd-K switches channels, chats, agents and projects, including restoring archived chats. Cmd-P provides
controls and setting toggles. Chat rows have archive/delete buttons; deletion requires confirmation.
Typing `/` offers Monitter actions such as new chat, settings and project selection, with applicable
Stop, terminal resume and read-only goal actions. Native harness commands need their own adapters;
unsupported commands stay unsent with an explanation. Use `//` for a literal slash message.

The Codex adapter follows Orbit's subprocess approach: `codex exec --json` for a first turn,
`codex exec resume` for later turns, prompts on stdin and structured events read from stdout.
The native session ID also lets you resume the task in a terminal. Attaching by ID does not take
over a running Codex Desktop or terminal process; finish its active turn first. Existing earlier
transcript import is separate from conversation context preserved by the Codex harness.

For Codex, read-only is the default. Workspace write is an explicit per-task choice at creation. Codex exec is
noninteractive: escalation requests are not approved automatically. If a tool is blocked, the error
is shown; Monitter does not present a pretend approval flow or use a bypass-all-permissions flag.

## Additional harnesses and activity

Source adapters now include Claude Code (`--print --output-format stream-json`), OpenCode
(`run --format json`), and Hermes (the installed TUI gateway through a standard-library Python
bridge). Each uses the host's native authentication and durable session IDs. Claude/OpenCode/Hermes
use **harness-configured permissions**, not Codex's OS sandbox. Monitter never enables bypass mode;
interactive approval requests are declined. Hermes needs its official source/venv installation
and Python 3; its full-duplex gateway supports richer activity than its one-shot CLI.

A compact in-chat panel shows actual computer-tool start/completion events and stops the owned
turn. Codex goals come from read-only `app-server thread/goal/get`; actual budgets/usage are displayed
only when reported. Hermes goal updates retain the gateway's reported text. This does not add a
Monitter goal scheduler or claim that every harness implements Codex's `/goal` command.

The existing local/Mira Codex integration has live validation. The new adapters currently have
parser/command/protocol tests; live account, tool, resume and cancellation checks remain to run.
This feature batch was packaged and installed after Alex returned, with Cmd-K/Cmd-P verified in
the native app. See [validation](docs/VALIDATION.md) for evidence and remaining live-harness checks.

## SSH

Use a normal SSH host alias or address, optional user/port/identity file, explicit remote executable
path and working folder. Monitter uses system SSH and its existing keys, agent, host configuration
and host-key checks. Password/host-key prompts are not handled in the app: complete the normal first
`ssh <host>` connection in Terminal, then test the host in Monitter.

Use the **same alias** that works in Terminal. `mira` and `mira.local` are different
SSH configuration and known-host lookup names, even when they resolve to the same
machine. Changing to an IP can likewise lose the configured user, key or trusted
host identity. Test the exact saved target noninteractively with
`ssh -o BatchMode=yes -o StrictHostKeyChecking=yes -o ConnectTimeout=8 <host> true`.
For a new host, verify its fingerprint through a trusted channel before accepting
it in Terminal; do not disable host-key checking or remove a changed key blindly.

ACP discovery and collaboration-helper staging report bounded SSH failure details.
Host-key errors occur before the agent starts; authentication failures require the
configured SSH key/agent, while a successful connection followed by “command not
found” needs the remote executable path. No failed chat is automatically resent.

The selected harness must already be installed and signed in on the remote host. Paths often differ from the Mac,
and a noninteractive SSH shell may not include npm's bin folder. For the authorized Mira test:

- SSH alias: `mira`
- Codex path: `/home/alex/.npm-global/bin/codex`
- Test working folder: `/home/alex/.local/share/monitter/smoke-workspace`

Remote hosts also need Python 3. Monitter runs a small inline supervisor using only its standard
library; it does not install a remote service. The supervisor owns each harness process group and
stops it when Monitter closes the control connection. The local Codex adapter runs directly.

## Channels and future versions

V1 channels live on this Mac. Choose the agents that should receive a message; messages do not
start an uncontrolled agent-to-agent loop. Delegated tasks retain a link to their parent task.

Multiplayer channels are explicitly deferred: a later version will allow colleagues or friends
to join channels and share selected agents. See [the future requirement](docs/FUTURE_MULTIPLAYER.md).
No invitations, shared credentials, collaboration server or iOS client is included in v1.

## Design and provenance

- Original supplied HTML: [design/reference.html](design/reference.html)
- Comparison of available views: [docs/DESIGN_VIEW_AUDIT.md](docs/DESIGN_VIEW_AUDIT.md)
- Architecture and command boundary: [docs/CONTRACT.md](docs/CONTRACT.md)
- Acceptance and current scope: [docs/BUILD_PLAN.md](docs/BUILD_PLAN.md)
- Orbit reference implementation: https://github.com/xinnaider/orbit
- Bundled IBM Plex font licenses: `static/licenses/`

This app is independently implemented; Orbit was studied for its CLI execution and resume patterns.
Build/test evidence and known limits are recorded in [docs/VALIDATION.md](docs/VALIDATION.md).

## Agent discovery and messaging

Open **monitter → Agent directory** to find agents by expertise, responsibility or skill. Edit an
agent's Collaboration profile to publish those details or turn collaboration off. New chats receive
the agent's profile and saved instructions.

Enabled Codex agents can discover peers, send messages and delegate work using Monitter tools.
Delegations create linked chats, use the recipient's own harness and host, and return actual results
to the source chat. Projects follow the delegation and select the recipient host's optional folder.
Run Detail shows the resulting message/delegation history. A message delivered to an active agent's
inbox is an acknowledgement of receipt, not a claim that its requested work is finished.

The runtime also has scoped Claude and OpenCode tool injection. Hermes currently supports receiving
routed tasks through its native adapter; callable Hermes collaboration tools are not yet exposed.
Host authentication remains with each native CLI. This runs while Monitter is open; no hosted
collaboration service, multiplayer sharing or iOS client is included.

The opt-in `scripts/collaboration-smoke.py --peer local|mira` exercises real Codex discovery,
delegation, peer inbox delivery, returned results and native-session resume in isolated app state.
Use `--coordinator mira --peer mira` to run both agents over SSH; `--model` selects a model supported
by both installed CLIs (the proof defaults to GPT-5.5 for Mira 0.130 compatibility). It consumes existing
Codex account usage and is not part of the offline unit test suite.
