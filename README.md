# Monitter

> [!IMPORTANT]
> **Early alpha.** I’m building Monitter for myself, but I think it has the potential to be useful to others. Expect rough edges and frequent changes. Testers, feedback, and contributions are welcome.

**Your agents, together.**

A cross-platform workspace for running, watching, and coordinating AI agents. Keep multiple conversations in view, give every project its own workspace, and work with agents on your computer or a remote machine over SSH.

Monitter brings your existing CLI harnesses into one interface. It uses their native sessions and authentication, with native adapters and support for the **Agent Client Protocol (ACP)**.

**Known to run on macOS, Android, and iOS. Windows and Linux are untested—we welcome testers and contributions.** The mobile apps are paired controllers: agents keep running on the connected desktop or its configured remote hosts.

![Two live agent conversations, project navigation, provider usage rings, and resource graphs](docs/screenshots/multipane-workspaces-draft.jpg)

*Real Monitter workspace: two agent chats side by side, with project navigation and monitoring in the sidebar.*

## A workspace for more than one agent

- **Multiple panes.** Keep agent conversations side by side, follow parallel work, and expand the pane that needs your attention.
- **Project workspaces.** Group chats by project, switch between saved workspaces, and configure working folders for local and SSH hosts. One project can involve several agents and machines.
- **Persistent conversations.** Keep native session IDs, transcripts, tabs, and execution settings together. Resume work without turning every follow-up into a new session.
- **Cross-agent collaboration.** Let supported agents discover peers, exchange messages, and delegate linked tasks. The receiving agent uses its own harness and host, and results return to the originating conversation.
- **Native and ACP harnesses.** Native adapters cover Codex, Claude Code, OpenCode, and Hermes. ACP connects compatible agents through a common protocol, locally or over SSH. Capabilities vary by harness; a successful connection is not a guarantee that every provider feature is supported.

## Full keyboard control

Keep your hands on the keyboard. The searchable **Controls palette** puts layouts, pane balancing, new chats, terminals, agent creation, and settings within reach. Use **↑ / ↓** to navigate, **Enter** to select, and **Esc** to dismiss.

On macOS, **⌘P** opens Controls and **⌘K** jumps between chats, agents, projects, and channels. Keyboard shortcuts make it easy to move around the workspace without hunting through menus.

![Searchable keyboard Controls palette with pane layouts and creation actions](docs/screenshots/keyboard-controls.png)

## Keep the work visible

![Live CPU and RAM history with the Monitter and harness process breakdown](docs/screenshots/resource-monitor-draft.jpg)

*Resource monitoring shows the app and its owned harness processes together.*

- **CPU and RAM graphs.** See Monitter and its owned harness processes in the sidebar, then open a resource breakdown with usage history and per-process details. Native resource monitoring is currently macOS-specific.
- **Provider usage rings.** Watch reported account allowances and reset windows for supported providers, including Codex, Claude, MiniMax, and OpenCode Go. Missing or stale readings are shown explicitly.
- **Context and goals.** See reported context usage and supported goal budgets alongside the conversation. The goal box stays above the composer while messages scroll.
- **Readable activity.** Inspect tool calls, available reasoning summaries, errors, approvals, and run details without losing the conversation. Scroll back to read while new output waits for you.
- **Git and terminals.** Inspect the current branch and changes, view tracked processes, and open a terminal in the chat's working folder.

## A garbage disposal for idle CLI processes

Persistent agents should not mean an ever-growing collection of idle processes.

Monitter's runtime collector retires eligible local harnesses after more than five minutes of inactivity, freeing their processes and disposable helpers while preserving the conversation and native session. Your next message resumes that session.

Cleanup is conservative: active work, queued messages, pending approvals, and uncertain background jobs keep a runtime alive. It only targets processes Monitter owns. Automatic retirement currently covers eligible macOS Codex app-server, Claude stream-json, and resumable ACP sessions; SSH runtimes are retained.

## Your agents can live on another machine

Configure a host, working folder, and harness executable, then control the agent over **SSH** from the same workspace.

Monitter uses your system SSH configuration, keys, and host-key checks. The harness must already be installed and authenticated on the remote machine. Native and ACP connections use that machine's session and credentials; Monitter does not replace the harness with its own model API.

## Take the conversation with you

- **Android and iOS controllers.** Pair a phone with the desktop to read conversations, send messages, and stop work. Execution stays on the host.
- **Browser sharing.** Share selected chats or project scope through an approval-based invitation. Guests join by name, and shared chat supports uploads and read-only model, effort, permissions, and context information where available.
- **Observer presence.** A sparkling eye shows connected observers with access to a chat; its tooltip lists their names.
- **Make it yours.** Light and dark themes, configurable accents, interface scaling, tab styles, and flexible pane layouts.

## Platform status

| Platform | Status |
| --- | --- |
| macOS | Confirmed running; main desktop development and validation platform. |
| Android | Confirmed running as a paired mobile controller. |
| iOS | Confirmed running as a paired mobile controller. |
| Windows | Untested. Build and compatibility reports welcome. |
| Linux | Untested. Build and compatibility reports welcome. |

This is an actively developed project, not a claim of feature parity across platforms. Desktop packaging, native process monitoring, and harness integration need more cross-platform testing. Provider support also depends on the installed CLI or ACP bridge and its version.

## Build and try it

The desktop app uses **Tauri 2, Rust, Svelte 5, and TypeScript**. Install Node/npm, Rust, and the native Tauri build dependencies for your operating system. On macOS, this includes Xcode command-line tools.

```sh
npm ci
npm run tauri dev
```

Agent execution requires a configured host with an installed, authenticated harness. `npm run dev` runs the browser design preview; it does not start native agents.

```sh
npm run check
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

## Experimental local model router

Monitter includes a deliberately narrow native MVP for evaluating whether a
Jev-guided route can use less expensive models without reducing coding-task
success. It is a library module (`src-tauri/src/model_router.rs`), a local CLI,
and an opt-in desktop feature attached to each saved harness. It cannot deploy,
auto-merge, spend money, alter credentials, or grant permissions.

In **Settings → Agents → Jev routing**, choose one mode per harness:

- **Off** is the default and makes no Jev call.
- **Recommend only** classifies a fresh task and records a compact local route
  trace, but keeps the operator's model selection.
- **Safe auto-route** may apply the operator's explicit tier-to-model mapping
  and a recommended reasoning level only when confidence is at least 0.50.

The composer marks enabled routing with a small Jev chip. An explicit composer
model selection always wins. A route requiring human review is stopped before
the harness is created; routing does not change the permission picker or
approve an effect. The desktop command is owner-local and stores only a prompt
fingerprint plus decision/evidence under Monitter app data `router-traces/`.

Build and run the credential-free mock path from the repository root:

```sh
cargo run --manifest-path src-tauri/Cargo.toml --bin harness -- \
  run "Find the failing test and fix the smallest underlying bug"

cargo run --manifest-path src-tauri/Cargo.toml --bin harness -- eval
```

After `cargo build --manifest-path src-tauri/Cargo.toml --bin harness`, the
same command is available as `src-tauri/target/debug/harness run "…"`. The
bundled `mock` provider is intentional: it exercises classification, isolated
worktree creation, evidence capture, reassessment, trace persistence, and the
evaluation report without a model account, shell execution, or live changes.

### Architecture

```text
prompt → Jev classifier → typed routing decision → isolated Git worktree
      → provider adapter → evidence → reassessment → finish/escalate/review
```

`MockJevClassifier` is a deterministic local stand-in. Its output has the
typed fields `task_kind`, `model_tier`, `reasoning_level`, `execution_mode`,
`permission_tier`, `confidence`, `rationale`, and `escalation_conditions`.
It follows the initial policy: low-cost answer/inspection first, balanced
routes for contained edits and ordinary bugs, strong for refactors, and
frontier inspection plus review for architecture or sensitive work.

An adapter implements `AgentProvider` and receives this exact request:

```rust
AgentRunRequest {
  prompt, workspace, model, reasoning_level, permission_tier, max_steps
}
```

Adapters return normalized evidence: executed-command descriptions, test
results, uncertainty, code/plan identification, steps used, and optional token
and cost fields. This keeps provider/model selection independent from
authority. A production adapter should bridge to Monitter's existing native
provider/runtime and approval controls; it must never accept a router decision
as a permission grant.

The MVP bundles two adapters: `mock` (safe, deterministic, no model call) and
`codex` (the installed, authenticated local Codex CLI). For Codex, model tier
maps to `MONITTER_ROUTER_FAST_MODEL`, `MONITTER_ROUTER_BALANCED_MODEL`,
`MONITTER_ROUTER_STRONG_MODEL`, and `MONITTER_ROUTER_FRONTIER_MODEL`; an
explicit `--model` wins. If a tier variable is absent, Codex uses its configured
default model and the trace says `codex-config-default`. The Codex adapter uses
only its `read-only` or `workspace-write` sandbox in the router-created Git
worktree, and does not pass `--approve-for-me` or either bypass flag.

### Live Jev classifier POC

The `run` command can use TypeSafe Jev for classification while retaining the
side-effect-free mock coding-agent provider. This makes a real HTTPS request to
Jev but does not call a coding model, execute shell commands, or make file
changes. It reads the Keychain-backed Monitter `JEV_API_KEY` secret first, then
falls back to `JEV_API_KEY` or `TYPESAFE_API_KEY` in the process environment.
The value is never added to an argv, trace, error, or renderer state.

```sh
cargo run --manifest-path src-tauri/Cargo.toml --bin harness -- \
  run "Find the failing test and fix the smallest underlying bug" --classifier jev
```

One `POST https://api.typesafe.ai/v1/systemone` call asks five parallel Choice
questions: task kind, model tier, reasoning level, execution mode, and minimum
permission tier. The router takes the lowest returned confidence as the
decision confidence and records TypeSafe-reported model, usage, cost when
available, and latency in the trace. A local sensitive-keyword preflight blocks
production, credentials, billing, deployment, deletion, permission, and
migration prompts before any request leaves Monitter.

To run the full local POC, combine the live Jev classifier with Codex:

```sh
MONITTER_ROUTER_BALANCED_MODEL="your-codex-model" \
  cargo run --manifest-path src-tauri/Cargo.toml --bin harness -- \
  run "Find the failing test and fix the smallest underlying bug" \
  --classifier jev --provider codex --max-steps 6
```

### Safety boundary

- Edit-capable runs require a Git workspace and receive a fresh detached
  worktree under the system temporary directory. The original workspace is not
  given to the adapter.
- Sensitive signals (production, deploy, credentials, billing, migrations,
  deletion, access control, and similar) become `human_review_required` and
  stop before the provider is called.
- Explicit `--model`, `--tier`, and `--reasoning-level` preferences are kept,
  but cannot remove a review gate or change the permission tier.
- Reassessment recommends escalation when confidence is below 0.50, code or a
  plan is missing, file/step budgets are crossed, tests fail, uncertainty is
  reported, or more than one top-level subsystem changes. The MVP never
  automatically retries on a stronger model.

Each run writes a compact JSON trace under `.monitter/router-traces/` (local
state, ignored by Git). The trace uses a SHA-256 prompt fingerprint rather than
storing the raw prompt, and records both decisions, selected provider/model,
permissions, evidence, files touched, test outcome, escalation, cost/tokens if
provided, outcome, and an optional `--user-correction` note. The live POC
records provider-reported tokens but does not invent a dollar cost when a
provider omits one.

`harness eval` compares the small representative fixture set against a fixed
strong route and the mock Jev route, reporting success, escalation, mock cost,
mock latency, and incorrect downgrades. These mock figures validate the
evaluation plumbing only; they are not claims about live model quality, price,
or latency. A later live evaluation should run the same tasks with explicitly
approved providers and preserve the resulting traces for review.

Platform build instructions:

- macOS: `npm run build:mac`, then `npm run install:mac`. For a local debug package, use `npm run build:mac:local` and `npm run install:mac -- --debug`.
- Android: [Android build and pairing guide](mobile-android/README.md).
- iOS: [iOS build and device guide](mobile-ios/README.md).

The macOS build is locally ad-hoc signed; this is not a notarized public installer.

For frontend development with an installed macOS app, run `npm run dev:app-ui`, then choose **Monitter → Load Hot-Reload UI**. Follow the LAN access pairing prompt if requested. Backend changes still require a rebuild. Run only one hot-reload server on port 18420 at a time.

## Testers welcome

Especially on Windows and Linux: try building the desktop app, connect a harness, and tell us what works and what breaks. Mobile device reports and native/ACP compatibility reports are welcome too.

When opening an issue, include your OS and version, Monitter commit, harness/version, whether the connection is local or SSH, and steps to reproduce. Remove credentials and private conversation content from logs and screenshots.

## More detail

- [Architecture and command boundary](docs/CONTRACT.md)
- [ACP adapter and compatibility](docs/ACP-ADAPTER.md)
- [Idle runtime retirement](docs/IDLE-RUNTIME-RETIREMENT.md)
- [Mobile controller](docs/MOBILE-CONTROLLER.md)
- [Validation evidence and known limits](docs/VALIDATION.md)
- [Shared browser interface](share-web/README.md)
- [Design reference](design/reference.html)

Monitter is independently implemented. [Orbit](https://github.com/xinnaider/orbit) informed early CLI execution and resume patterns. Bundled font licenses are in `static/licenses/`.

The built-in collaboration MCP runs in Rust over loopback HTTP. Local harnesses connect
directly; SSH harnesses use an owned reverse tunnel. Codex, Claude, OpenCode and
compatible ACP sessions receive scoped collaboration tools. The Python MCP relay
has been removed; other Python integrations remain. See [the Rust HTTP MCP notes](docs/RUST-HTTP-MCP.md).
