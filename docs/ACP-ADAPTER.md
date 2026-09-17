# Generic ACP adapter

Implementation target: ACP v1, with capability negotiation. This is a new
transport, not a collection of Gemini/Pi/OpenCode parsers. Any compatible executable
can be configured with an argument vector. Presets are conveniences, not an
allowlist of agents the transport accepts.

## Setup and discovery

- Agent setup offers a searchable catalog, installed candidates first, and Custom
  ACP agent. Preserve the existing native integrations alongside ACP.
- Detection checks known executable locations on the selected host, not an
  unrestricted disk scan. Finding a file is not proof of ACP support or login.
- Show distinct detected, not found, and handshake-verified states. Verification
  is explicit, bounded, and does not create a session or send a model prompt.
- Commands and arguments are separate editable values. No shell evaluation,
  implicit package installation, automatic downloads, or credentials in arguments.
- The official registry is a source of reviewed preset metadata, never a source
  of automatically executable instructions. An installed `npx` is not evidence
  that an agent package is installed.
- Pi's native RPC is not ACP. Offer an installed Pi ACP bridge, clearly labelled,
  rather than sending ACP messages to `pi --mode rpc`.

## State and compatibility

An ACP agent has provider `acp` and an explicit command/argument configuration.
New tasks capture that configuration; edits affect future chats only. Native
session ownership includes the transport configuration, host and session ID.
Existing OpenCode tasks keep their current transport and native history. The
OpenCode ACP preset is an explicit choice for new agents/chats, not an automatic
migration or silent fallback.

One owned full-duplex process serves a chat across turns. New sessions follow
initialize -> session/new -> session/prompt. Reconnection uses session/resume
when advertised, or session/load when advertised. Replay from load is not added
again to Monitter's existing transcript. Unsupported recovery fails visibly;
it never silently creates a replacement conversation or replays an uncertain turn.

Only supported capabilities are advertised. The initial client does not claim
filesystem, terminal, boolean config, or elicitation capabilities before those
callbacks and their permission boundaries exist. Mandatory permission requests
use durable owner approvals, exact request/option IDs and one-time decisions.
Unsupported or expired requests fail closed, never selecting allow_always.

Identity settings and the actual first user message are delivered together as
prompt content; ACP does not imply a universal system-prompt parameter. Model,
mode and configuration choices must come from the agent's advertised session
state. No brand-specific flags are silently appended to custom commands.
The existing model picker reads advertised model choices from a resident ACP
session. Before a session exists, use the agent's default or an explicitly entered
model ID; a nonempty ID must be advertised before Monitter sends the prompt.
Configuration changes are acknowledged before the next prompt is released.

When collaboration is enabled, Monitter passes its Rust HTTP MCP endpoint in
`mcpServers`. The agent must advertise HTTP MCP support at initialization. Tokens
are ephemeral Authorization headers inside the private session protocol frame,
not launcher arguments or persisted agent settings. SSH uses the existing
loopback-only reverse tunnel, which is stopped on owned teardown. No MCP helper
is launched or copied to the remote host.

Diagnostics must not block sends or token streaming. Bound protocol frames,
pending requests, text accumulation and diagnostic output. Unknown optional
notifications do not terminate a compatible session; unsupported requests receive
JSON-RPC errors. Cancellation resolves pending permissions as cancelled and cleans
up only the owned process tree.
Assistant text and reasoning are coalesced at approximately 100 ms intervals;
the first message item is immediate and final completion flushes remaining text.
ACP `messageId` groups chunks without conflating distinct replies. Supported
inline JPEG/PNG/WebP images attach to the matching message (four images per
message, sixteen per turn, 512 KiB inline data URL limit each). Unsupported output content is
identified visibly, never automatically fetched or silently executed.

## Current limits

- Compatibility means ACP v1 over stdio, locally or through SSH. A custom agent
  must expose that protocol itself or through an installed bridge. It is not a
  promise that arbitrary non-ACP CLI output will work.
- Authentication stays with the agent's existing CLI/environment. The picker
  does not perform OAuth or install packages. Pi needs a separate ACP bridge.
- Optional client filesystem, terminal and elicitation callbacks are not
  advertised. The agent can still use its own tools and permission policy.
- Model choices are available after session initialization. Generic mode and
  non-model configuration controls, audio playback and automatic resource-link
  loading are not yet exposed in Monitter.
- Existing OpenCode/native sessions are never automatically migrated to ACP.

## Verification plan

Deterministic subprocess fixtures cover LF framing/Unicode, version negotiation,
first and subsequent prompts, streamed text and tools, request IDs, permission
approve/deny/cancel, capabilities, timeout/process loss, replay suppression,
configuration snapshots, host-aware detection and injection-safe command launch.
Fixtures are test-only and are never displayed as real agents in the app.

Real installed-agent verification is reported separately from fixture coverage.
Initialize-only probes do not demonstrate authenticated model turns. No live app
configuration, installation or merge is part of this feature's implementation.

## Primary references

- [Protocol overview](https://agentclientprotocol.com/protocol/v1/overview)
- [Initialization](https://agentclientprotocol.com/protocol/v1/initialization)
- [Session setup](https://agentclientprotocol.com/protocol/v1/session-setup)
- [Prompt turns](https://agentclientprotocol.com/protocol/v1/prompt-turn)
- [Tool calls and permissions](https://agentclientprotocol.com/protocol/v1/tool-calls)
- [Session configuration](https://agentclientprotocol.com/protocol/v1/session-config-options)
- [Registry source](https://github.com/agentclientprotocol/registry)

## Verified development baseline

- Rust library regressions: 218 passed, five opt-in live tests skipped.
- Chromium and touch WebKit: preset search, host-specific detection, literal
  custom arguments, saving, explicit verification and stale-result invalidation.
- Controller/shared projections: launcher settings remain private.
- Remote supervisor: exact argument handling, disconnect cleanup, resistant child
  cleanup and blocked-stdin termination.
- Svelte/TypeScript check: zero errors or warnings; private production web build
  successful. No merge, installation or live LAN asset publication performed.

Deterministic fixtures verify the transport and ownership boundaries; real
provider sign-in and billable model turns remain separate, opt-in verification.

## Repeatable checks

Run these from the isolated feature worktree. `npm run build` also publishes LAN
assets on macOS, so use the direct Vite command for private verification:

```sh
npm run check
node node_modules/vite/bin/vite.js build
node node_modules/vite/bin/vite.js preview --host 127.0.0.1 --port 18458 --strictPort
# In a separate terminal:
node scripts/ui-acp-setup-smoke.mjs
node scripts/controller-protocol-test.mjs
python3 scripts/test-acp-remote.py
CARGO_TARGET_DIR="$PWD/.cargo-target" cargo test --manifest-path src-tauri/Cargo.toml --lib
```

`scripts/acp-handshake.mjs /absolute/path/to/agent <ACP arguments>` performs an
opt-in, initialize-only installed-agent check without a session or model request.
Gemini CLI 0.59.0 and OpenCode 1.18.30 completed that handshake during development;
these checks do not establish authenticated model-turn or Pi-bridge compatibility.
