# `mona` ACP server design

Status: **Approved, ready for Phase 1 spike**. Targets the `mona` fork of `jcode v0.86.0` once renamed and re-homed. Complements `docs/ACP-ADAPTER.md` (the Monitter-side ACP *client*); this document is about exposing the fork as an ACP *server*.

## Naming (locked 2026-09-20)

| Concept | Name | Rationale |
|---|---|---|
| User-facing binary | `mona` | Short, mnemonic for **Mon**itter **A**gent. Easy to type, distinct from `jcode`. |
| Crate root | `mona` (Cargo package name) | Matches the binary. |
| Workspace crates | `mona-*` | `mona-base`, `mona-app-core`, `mona-agent-runtime`, `mona-sdk`, `mona-acp`, `mona-jev`, `mona-provider-*`, etc. |
| Internal lib target | `mona` | Same as root crate. |
| Home directory | `~/.mona/` | Short, no leading dot confusion. |
| Auth files | `~/.mona/openai-auth.json`, `~/.mona/anthropic-auth.json`, etc. | |
| Legacy daemon socket | `~/.mona/mona.sock` | |
| Harness API bridge socket | `~/.mona/api.sock` | |
| Env vars | `MONA_SOCKET`, `MONA_API_SOCKET`, `MONA_ALLOW_CODEX_LEGACY_AUTH` | |
| Internal provider strings | `openai`, `claude`, `gemini`, `copilot`, `openrouter`, etc. **UNCHANGED** | These are model family identifiers, not harness names. The `jcode` → `mona` rename only affects the harness, not the providers. |
| Wire protocol request types | `SetModel`, `SetReasoningEffort`, `Cancel`, etc. **UNCHANGED** | These are the stable ACP-compatible API surface. Don't rename for branding. |
| Self-dev gate crate | `mona-selfdev-types`, looking for `crates/mona-desktop-ui/` with `name = "mona-desktop"` | Naturally inert in Monitter task directories after rename. |
| GitHub fork | `github.com/soyrex/mona` | |
| Upstream reference | `github.com/1jehuang/jcode` | Forked, renamed, with periodic re-sync. |
| LICENSE file | Verbatim MIT license from `jcode`, plus a `MONA_NOTICE.md` at the fork root | Legal requirement (MIT license preservation). |

The user-facing spelling of `mona` is **always lowercase** — never `MONA`, never `Mona`. Matches the spirit of `jcode`.

## Why this exists

Monitter's desktop, mobile, and share-web clients already speak ACP to remote
harnesses (`src-tauri/src/acp_transport.rs`, `acp_runtime.rs`,
`acp_session_config.rs`, `acp_discovery.rs`, `acp_protocol.rs`). Today the
remote harnesses are third-party CLIs (OpenCode ACP bridge, future Codex ACP,
Pi ACP bridge, Gemini, etc.).

The `mona` fork is meant to be the harness Monitter owns. It needs
to be **drivable from any surface that already speaks ACP** so that mobile and
share-web clients gain per-turn Jev routing without writing new transport
code on their side.

Two integration surfaces exist for `mona`:

1. **`mona serve`** over `~/.mona/api.sock` (Unix) or named
   pipe (Windows). Custom JSON protocol exposed via `mona-sdk`.
   Owned by Monitter desktop only.
2. **`mona-acp`** over stdio. The same Agent loop, exposed as an
   ACP stdio server. Any ACP client can drive it. **This document.**

Surface 1 ships first (Phase 1, see `MODEL-ROUTER-HARNESS-PLAN.md`). Surface
2 is Phase 2 and is the focus of this document.

## Wire contract

`agent-client-protocol = "=0.10.4"` with the `unstable_session_model`,
`unstable_session_resume`, `unstable_session_usage` features — the same
feature set `jcode` already depends on as a Grok Build client. The fork
already speaks these messages; we flip the role from client to server.

### Lifecycle

```
client                                  mona-acp
  | -- initialize(protocol_version=1) --> |
  | <-- InitializeResponse{             -- |
  |     capabilities: {                 -- |
  |       session_model: true,          -- |
  |       session_resume: true,         -- |
  |       session_usage: true,          -- |
  |       jev_routing: true,            -- |  ← NEW (see "Extensions")
  |       auth_methods: [...],          -- |
  |     },                              -- |
  |     auth_methods: [...],            -- |
  |     agent_info: { name, version }   -- |
  |   }                                  -- |
  | -- authenticate(method_id) --------> |  (required when InitializeResponse indicates auth required)
  | <-- AuthResponse{ ok: true } ------ |
  | -- session/new ----------------------> |
  | <-- Session{ id, model, ... } ------ |
  | -- session/prompt(text) ------------> |
  | <-- stream SessionUpdate{...} ------ |
  | <-- PromptComplete{stop_reason} ---- |
  | -- session/set_model{model} -------> |  ← unstable_session_model
  | <-- ModelUpdated{model,effort} ---- |
  | -- session/set_reasoning_effort{--> |  ← NEW (Monitter-side extension)
  |       effort}>                      -- |
  | <-- ReasoningEffortUpdated{...} --- |
  | -- session/cancel ------------------> |  ← soft then hard
  | -- session/resume{id} --------------> |  ← unstable_session_resume
```

### Extensions beyond upstream ACP

Three additions are specific to `mona`:

1. **`session/set_reasoning_effort`** — ACP has `unstable_session_model` but
   no standard method for reasoning effort. We add it as a Monitter extension,
   negotiated in `InitializeResponse.capabilities.reasoning_effort: bool`.
   Falls back to provider's native mechanism (`X-Claude-Effort` for Anthropic,
   `reasoning_effort` for OpenAI Responses, etc.) when the underlying provider
   doesn't accept arbitrary values.

2. **`session/jev_route`** — fire-and-forget advisory routing decision from
   the client. The server may accept, ignore, or override per its own
   `JevRoutePolicy`. In Phase 2 we accept it as a **hint**, not as an
   authoritative instruction. The server's own Jev classifier is the
   source of truth. The client's hint is used only when the server is in
   `recommend` mode (`JevRoutingMode::Recommend`) instead of `safe_auto`.

3. **`session/router_trace`** — read-only notification. After every
   `session/set_model` or `session/set_reasoning_effort`, the server
   emits a `RouterTrace` event containing the Jev decision that caused
   the change: `tier`, `reasoning_level`, `confidence`, `trigger`
   (`initial_prompt` | `turn_reclassified` | `user_override` |
   `cooldown_expired` | `escalation_followed`).

The `jev_routing: bool` capability is **only `true`** when the server is
configured with at least one authenticated `jcode` provider and a working
Jev credential. Otherwise the server reports `false` and the
`session/set_model` semantics fall back to plain ACP (the client picks
the model, server honors it, no Jev decision is involved).

### Auth handshake

`mona` already implements OAuth for ~15 providers
(`crates/jcode-base/src/auth/` in the fork). When the fork is started
with `mona-acp serve`, it inherits the existing `~/.mona/`
auth state. If a provider is unauthenticated, the server reports the
auth method as `unauthenticated` and refuses to load that provider.

The server advertises `auth_methods` in `InitializeResponse`. Each entry
has `id`, `display_name`, `status` (`authenticated` | `unauthenticated` |
`expired`), and optional `description`. This lets the client's UI show
"Connect Claude" / "Connect OpenAI" buttons and call `authenticate` with
the right method_id.

### Session resume

`unstable_session_resume` works because the fork's `Agent::run_turn`
already accepts `resume_session_id: Option<&str>`. We just need to map
the ACP `session/resume` request onto the existing
`Agent::attach_session()` path (`crates/jcode-app-core/src/agent.rs:457`).

### Cancellation

`session/cancel` maps onto the existing `SoftInterruptMessage` /
`InterruptSignal` infrastructure (`crates/jcode-agent-runtime/src/lib.rs:5–121`).
Two-phase:
- First cancel: soft interrupt (lets the agent finish its current tool call).
- Second cancel (within 5s): hard kill of the underlying provider request.

This matches the behavior of the existing `jcode` TUI.

## Crate layout

New crate: `crates/mona-acp/`

```
crates/mona-acp/
├── Cargo.toml
├── src/
│   ├── lib.rs              # crate root; re-exports server + router glue
│   ├── server.rs           # ACP stdio server loop
│   ├── routes/
│   │   ├── mod.rs
│   │   ├── initialize.rs   # capabilities negotiation
│   │   ├── authenticate.rs # provider-level auth
│   │   ├── session.rs      # session/new, session/prompt, session/resume, session/cancel
│   │   ├── model.rs        # session/set_model + unstable routing
│   │   ├── effort.rs       # session/set_reasoning_effort (Monitter ext)
│   │   └── trace.rs        # router_trace stream
│   ├── policy.rs           # JevRoutePolicy (off | recommend | safe_auto | per_turn)
│   ├── cooldown.rs         # per-session tier-change cooldown (≤1 per N turns)
│   ├── bridge.rs           # glue between ACP request stream and Agent::run_turn
│   └── tests/
│       ├── initialize_test.rs
│       ├── session_test.rs
│       ├── model_swap_test.rs
│       └── policy_test.rs
└── tests/
    └── end_to_end.rs       # spawns the binary, speaks ACP, asserts events
```

### Crate dependencies

```toml
[dependencies]
agent-client-protocol = { version = "=0.10.4",
                          features = ["unstable_session_model",
                                      "unstable_session_resume",
                                      "unstable_session_usage"] }
mona-base = { path = "../mona-base" }
mona-app-core = { path = "../mona-app-core" }
mona-provider-core = { path = "../mona-provider-core" }
mona-jev = { path = "../mona-jev" }
mona-message-types = { path = "../mona-message-types" }
mona-agent-runtime = { path = "../mona-agent-runtime" }

anyhow = "1"
async-trait = "0.1"
futures = "0.3"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tokio-stream = "0.1"
tracing = "0.1"
```

`mona-jev` is the new crate that hosts the Jev classifier
inside the fork. It re-implements the `JevClassifier` trait
(Monitter-side reference: `src-tauri/src/model_router.rs:380`) with the
same Keychain-backed API key contract. The trait shape:

```rust
#[async_trait]
pub trait JevClassifier: Send + Sync {
    async fn classify(&self, request: &JevClassifyRequest) -> Result<JevRoutePlan>;
}

pub struct JevClassifyRequest {
    pub prompt: String,
    pub recent_messages: Vec<JevMessage>,
    pub last_turn_outcome: Option<JevTurnOutcome>,
    pub available_models: Vec<String>,
    pub available_efforts: Vec<String>,
}

pub struct JevRoutePlan {
    pub tier: ModelTier,                 // fast | balanced | strong | frontier
    pub effort: Option<String>,          // none | minimal | low | medium | high | xhigh | max
    pub reasoning_level: ReasoningLevel, // low | standard | deep
    pub execution_mode: ExecutionMode,   // plan | confirm | autopilot
    pub permission_tier: PermissionTier, // read | write_local | write_remote | destructive
    pub confidence: f32,                 // 0.0 - 1.0
    pub rationale: String,
    pub sensitive: bool,                 // hard-overrides routing
    pub trace_id: Uuid,
}

pub enum JevTurnOutcome {
    Passed,
    Failed { reason: String },
    Uncertain { reason: String },
}
```

The trace is persisted to `~/.mona/router-traces/{trace_id}.json`
in the same shape as Monitter's `.monitter/router-traces/`
(`src-tauri/src/model_router.rs:1293–1315`), keeping the two schemas
compatible so the Monitter UI can render either.

## Server loop

```rust
// crates/mona-acp/src/server.rs (sketch)

pub async fn serve(
    agent: Arc<Mutex<Agent>>,
    classifier: Arc<dyn JevClassifier>,
    policy: JevRoutePolicy,
    input: impl AsyncRead + Unpin,
    output: impl AsyncWrite + Unpin,
) -> Result<()> {
    let conn = acp::AgentSideConnection::new(
        MonitterAgentSide::new(agent, classifier, policy),
        input, output,
    );
    conn.serve().await
}

struct MonitterAgentSide {
    agent: Arc<Mutex<Agent>>,
    classifier: Arc<dyn JevClassifier>,
    policy: JevRoutePolicy,
    sessions: HashMap<SessionId, SessionState>,
}

#[async_trait]
impl acp::Agent for MonitterAgentSide {
    async fn initialize(&self, req: InitializeRequest) -> Result<InitializeResponse, acp::Error> {
        // Advertise capabilities based on policy + classifier availability.
        // ...
    }

    async fn authenticate(&self, req: AuthenticateRequest) -> Result<AuthResponse, acp::Error> { ... }
    async fn new_session(&self, req: NewSessionRequest) -> Result<NewSessionResponse, acp::Error> { ... }
    async fn prompt(&self, req: PromptRequest) -> Result<PromptResponse, acp::Error> { ... }
    async fn cancel(&self, req: CancelNotification) -> Result<(), acp::Error> { ... }
    async fn resume_session(&self, req: ResumeSessionRequest) -> Result<ResumeSessionResponse, acp::Error> { ... }

    // Monitter extensions (negotiated via capabilities)
    async fn set_model(&self, req: SetModelRequest) -> Result<SetModelResponse, acp::Error> { ... }
    async fn set_reasoning_effort(&self, req: SetReasoningEffortRequest) -> Result<SetReasoningEffortResponse, acp::Error> { ... }
}
```

The actual implementation of `set_model` and `set_reasoning_effort` is
where Jev routing hooks in.

## Routing hook (the important part)

```rust
// crates/mona-acp/src/routes/model.rs (sketch)

async fn set_model(
    state: &mut ServerState,
    session_id: &SessionId,
    requested_model: &str,
) -> Result<SetModelResponse> {
    // 1. Sanity: does the requested model exist?
    let available = state.agent.available_models().await?;
    if !available.iter().any(|m| m == requested_model) {
        return Err(invalid_request(format!("model {requested_model} not available")));
    }

    // 2. Apply cooldown gate: refuse if too soon since the last swap
    let session = state.session_mut(session_id)?;
    if session.cooldown_active() {
        return Err(throttled_request("tier change in cooldown; try again in N turns"));
    }

    // 3. Safety gates (these come from the existing Jev safety contract)
    let plan = if matches!(state.policy, JevRoutePolicy::SafeAuto) {
        // SafeAuto honors the request without re-classifying
        JevRoutePlan::passthrough(requested_model)
    } else if matches!(state.policy, JevRoutePolicy::PerTurn) {
        // PerTurn re-classifies with full context
        Some(state.classifier.classify(&JevClassifyRequest {
            prompt: session.last_user_prompt.clone().unwrap_or_default(),
            recent_messages: session.recent_messages(4),
            last_turn_outcome: session.last_outcome(),
            available_models: available.clone(),
            available_efforts: state.agent.available_efforts(),
        }).await?)
    } else {
        // Recommend / Off — accept the request verbatim
        None
    };

    // 4. Apply the model via the existing provider trait
    state.agent.set_model_with_auth_refresh(requested_model).await?;

    // 5. Persist + emit the trace
    let trace = RouterTrace {
        session_id: session_id.clone(),
        trace_id: plan.as_ref().map(|p| p.trace_id).unwrap_or_else(Uuid::new_v4),
        requested_model: requested_model.to_string(),
        applied_model: requested_model.to_string(),
        tier: plan.as_ref().map(|p| p.tier),
        confidence: plan.as_ref().map(|p| p.confidence),
        trigger: RouterTrigger::ClientRequested,
        rationale: plan.as_ref().map(|p| p.rationale.clone()),
        occurred_at: Utc::now(),
    };
    state.trace_persist(&trace);
    state.session.broadcast(RouterTraceEvent::new(trace.clone()));

    // 6. Record the swap for cooldown
    session.record_swap();

    Ok(SetModelResponse { model: requested_model.into(), trace })
}
```

The trace event is sent to **every attached client** — same semantics
that `jcode-sdk`'s `model_info` event has today.

## Cooldown policy

```rust
// crates/mona-acp/src/cooldown.rs

pub struct CooldownPolicy {
    /// Minimum number of user turns between tier swaps.
    pub min_turns_between_swaps: u32,        // default: 2
    /// Maximum number of tier swaps allowed in a single session.
    pub max_swaps_per_session: u32,          // default: 8
    /// Confidence floor below which a Jev re-route is ignored.
    pub confidence_floor: f32,               // default: 0.5
    /// Hard limit on escalations per session (strong → frontier).
    pub max_escalations_per_session: u32,    // default: 3
}

impl Default for CooldownPolicy {
    fn default() -> Self {
        Self {
            min_turns_between_swaps: 2,
            max_swaps_per_session: 8,
            confidence_floor: 0.5,
            max_escalations_per_session: 3,
        }
    }
}
```

These numbers are operator-tunable. They live in the existing
`[agent]` section of `~/.mona/config.toml`:

```toml
[agent.acp_server]
enabled = true
socket = "~/.mona/api.sock"  # for the serve-mode surface (Phase 1)
policy = "per_turn"  # off | recommend | safe_auto | per_turn

[agent.acp_server.cooldown]
min_turns_between_swaps = 2
max_swaps_per_session = 8
confidence_floor = 0.5
max_escalations_per_session = 3
```

## Failure modes

| Failure | Behavior |
|---|---|
| Jev classifier unreachable (network down) | Use last cached plan. If none, fall back to provider default model. Emit `router_trace` with `trigger: Unavailable`. |
| `set_model` returns `Err` from the provider | Re-attempt with `set_model_with_auth_refresh`. If still fails, surface the error as an ACP `internal_error` response. Don't silently keep the old model. |
| `session/set_reasoning_effort` value not supported by current provider | Return ACP `invalid_request` with `{ accepted_values: [...] }`. |
| Cooldown violated (client retries too fast) | Return ACP `request_throttled` with `{ retry_after_turns: N }`. |
| Sensitive prompt detected by Jev | Refuse `session/new` with `permission_required` and request human approval. Mirrors `src-tauri/src/model_router.rs:1112–1126` (the existing `Overevidence::SensitivePrompt` gate). |
| Provider auth expired mid-session | Re-read auth from disk; if still expired, return `unauthenticated` and request re-auth via `authenticate`. |
| ACP client version mismatch | Negotiate down to the lower of client/server `protocol_version`. Log the mismatch. |

## Failure-mode verification

Each row above maps to one test in `crates/mona-acp/tests/end_to_end.rs`:

- `jev_unreachable_uses_cached_plan`
- `provider_set_model_failure_returns_internal_error`
- `unsupported_effort_returns_invalid_request_with_accepted_values`
- `cooldown_returns_throttled_with_retry_after`
- `sensitive_prompt_returns_permission_required`
- `auth_expired_returns_unauthenticated`
- `protocol_version_negotiation_downgrades_safely`

## Migration path: server doesn't break existing clients

`mona-acp` advertises the full capability set, but every
extension is **negotiated** in `InitializeResponse`. A vanilla ACP
client (e.g. a future Codex ACP, or the OpenCode ACP bridge) that
doesn't know about `unstable_session_model` would just not send
`session/set_model`. The server would still serve it correctly.

Conversely, a Monitter client connecting to a non-`mona` ACP
server would attempt `session/set_model` and get `unknown_method`,
falling back to creating a new session with the desired model
(equivalent to today's `accept_send` behavior).

## Integration with Monitter's existing ACP discovery

When `mona-acp` is installed, Monitter's `acp_discovery.rs`
should:

1. Detect the binary at well-known paths (Homebrew, `/usr/local/bin`,
   `~/.mona/bin/`).
2. Probe via `initialize` (no auth, no `session/new`).
3. If the response includes `jev_routing: true` in capabilities,
   surface a "Per-turn Jev routing" badge in the agent picker.

The agent card UI in `AppSurface.svelte` reuses the existing
`JevRoutingMode` chip from `AppSurface.svelte:2832`. No new UI surface
needed — the same chip displays "Per-turn Jev routing" or "Recommended"
depending on the negotiated policy.

## Open questions for the spike

1. **ACP server runtime: spawn-on-demand or persistent?** Persistent
   (`mona-acp serve` runs as a background process) is
   simpler for repeated session creation but adds lifecycle management.
   Spawn-per-session is easier to reason about but slower.

2. **Socket transport in addition to stdio?** ACP stdio is the universal
   ACP transport, but Monitter's existing `acp_transport.rs` supports
   SSH stdio (line 26) and local stdio (line 23). Phase 2 could expose
   `mona-acp` over a Unix socket (`~/.mona/api.sock`)
   with stdio framing, giving mobile clients a clean remote path without
   SSH. Decision: defer to Phase 3 unless mobile demands it.

3. **Multi-client sessions.** The fork's `Agent` is per-session but
   multiple ACP clients can `attach_session` to the same session_id
   (the existing `Agent::attach_session` path). This is supported by
   the SDK's `model_info` push semantics. Worth verifying that the
   `RouterTraceEvent` reaches all attached clients correctly.

4. **`session/jev_route` advisory hint.** Should the server honor the
   client's hint when the server's policy is `recommend` (not
   `safe_auto`)? My recommendation: yes, but only when the server's
   confidence on its own classification is below `confidence_floor`.
   This lets a smart client (Monitter UI) steer routing when the
   server-side Jev is uncertain.

## What this design does not include

- **Tool call routing.** `session/set_model` mid-prompt would lose
  tool state. Phase 3: per-tool-call routing via sub-agent delegation
  (`mona-delegate` tool that spawns a fast-model child task
  for a specific tool and returns the result).
- **Multi-agent trees.** ACP's `session/fork` is supported, but
  Monitter's subagent UI (`AppSurface.svelte` subagent visor) expects
  a different event shape. Phase 3.
- **Heterogeneous model contexts.** Different parts of the same prompt
  routed to different models, then merged. Hard. Phase 4 if ever.

## Effort estimate (Phase 2 only)

| Workstream | Effort | Confidence |
|---|---|---|
| `mona-jev` crate (port `JevClassifier` from Monitter's `model_router.rs`) | 1 week | high |
| `mona-acp` crate: server loop, route handlers, capability negotiation | 2 weeks | medium-high — depends on `agent-client-protocol` 0.10.4 server-side API surface (it's used as a client elsewhere in `jcode`; the server half needs careful reading) |
| Cooldown + safety gates + trace persistence | 1 week | high |
| Wire into `mona-app-core::Agent::run_turn` so the Jev hook fires per turn | 1 week | medium — the existing `set_model_with_auth_refresh` call site is in `crates/jcode-app-core/src/agent.rs:489`; the per-turn variant needs to be similarly placed |
| ACP discovery hook in Monitter's `acp_discovery.rs` | 2 days | high |
| UI: badge + capability-driven picker card | 2 days | high |
| End-to-end test harness + integration with Monitter desktop | 1 week | medium |
| **Total** | **~5–7 engineer-weeks** | |

This is on top of the Phase 1 estimate (~5 weeks to land the fork with
`mona serve`).

## Combined Phase 1 + Phase 2

~10–12 engineer-weeks for an end-to-end `mona` with both
the SDK surface and the ACP surface. Mobile and share-web clients
gain per-turn Jev routing through the ACP path without writing new
transport code. Desktop uses whichever surface is more convenient per
case (SDK for typed Rust integration, ACP for symmetry with other
remote harnesses).
