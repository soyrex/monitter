# Phase 2: `mona-acp` crate + per-turn Jev routing

Status: **Ready to start**. Phase 1 (`feature/jcode-to-mona-rename` branch on `soyrex/mona`) shipped; this plan is the next milestone.

**Phase 2 provider set**: `codex` (OpenAI), `claude` (Anthropic), `minimax` (MiniMax). All three already wired in upstream and inherited by the rename.

## Why this exists

Phase 1 stripped the CLI surface to one command (`mona acp`) but the underlying ACP implementation is upstream's. Phase 2 builds a **Monitter-owned** ACP server in a new crate `crates/mona-acp/`, with per-turn Jev routing embedded inside the agent loop. Phase 2 is what makes `mona` worth shipping as a Monitter harness rather than just a renamed `jcode`.

## What Phase 2 ships

### New crate: `crates/mona-acp/`

```
crates/mona-acp/
├── Cargo.toml                  # depends on mona_app_core, mona_base, mona_provider_core,
│                               # mona_jev, agent-client-protocol 0.10.4
├── src/
│   ├── lib.rs                  # crate root; re-exports server + policy
│   ├── server.rs               # ACP stdio server loop (agent-side connection)
│   ├── routes/
│   │   ├── mod.rs
│   │   ├── initialize.rs       # capabilities negotiation; advertises `jev_routing: bool`
│   │   ├── authenticate.rs     # provider-level auth (delegates to mona_base::auth)
│   │   ├── session.rs          # session/new, session/prompt, session/resume, session/cancel
│   │   ├── model.rs            # session/set_model + Jev routing hook
│   │   ├── effort.rs           # session/set_reasoning_effort (Monitter extension)
│   │   └── trace.rs            # router_trace push event (Monitter extension)
│   ├── policy.rs               # JevRoutePolicy: off | recommend | safe_auto | per_turn
│   ├── cooldown.rs             # per-session tier-change cooldown
│   ├── bridge.rs               # glue between ACP request stream and Agent::run_turn
│   ├── provider_whitelist.rs   # allowlist: codex | claude | minimax
│   └── tests/
│       ├── initialize_test.rs
│       ├── session_test.rs
│       ├── model_swap_test.rs
│       └── policy_test.rs
└── tests/
    └── end_to_end.rs           # spawns the binary, speaks ACP, asserts events
```

### New crate: `crates/mona-jev/`

A Monitter-owned Jev classifier trait + a Keychain-backed implementation. Upstream has `crates/mona-base/src/jev.rs` (1,109 lines, already using `MONA_*` env vars) but it's tightly coupled to upstream's macOS notification broker and the Jcode Cloud/Jade service. We want a clean re-implementation focused on per-turn routing.

```
crates/mona-jev/
├── Cargo.toml                  # depends on mona_provider_core (for catalog)
├── src/
│   ├── lib.rs                  # JevClassifier trait + plan types
│   ├── mock.rs                 # in-memory classifier for tests
│   ├── live.rs                 # HTTP client to JEV endpoint (Keychain-backed)
│   ├── classifier.rs           # classification logic (rule-based + Jev hybrid)
│   ├── safety.rs               # sensitive-prompt gate, never-widen-permission checks
│   └── tests/
│       └── classify_test.rs
└── tests/
    └── safety_gates_test.rs
```

The trait shape (mirrored from Monitter's `src-tauri/src/model_router.rs:380`):

```rust
#[async_trait]
pub trait JevClassifier: Send + Sync {
    async fn classify(&self, req: &JevClassifyRequest) -> Result<JevRoutePlan>;
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
    pub sensitive: bool,
    pub trace_id: Uuid,
}
```

### Per-turn Jev routing hook

Inserted into `Agent::run_turn` (or whichever call site in `mona-app-core/src/agent/turn_execution.rs` drives a single turn's `provider.complete()`). Pseudocode:

```rust
async fn run_turn_with_jev(&mut self, user_message: &str) -> Result<()> {
    // 1. Build classify request from session state
    let req = JevClassifyRequest {
        prompt: user_message.to_string(),
        recent_messages: self.recent_messages(4),
        last_turn_outcome: self.last_outcome(),
        available_models: self.available_models(),
        available_efforts: self.available_efforts(),
    };

    // 2. Classify (or skip if policy says so)
    let plan = match self.policy {
        JevRoutePolicy::Off => return self.run_turn_inner(user_message).await,
        JevRoutePolicy::Recommend | SafeAuto | PerTurn => {
            self.classifier.classify(&req).await?
        }
    };

    // 3. Safety gates (hard-overrides)
    if plan.sensitive {
        return Err(anyhow!("Sensitive prompt; routing to human review required."));
    }

    // 4. Cooldown gate
    if self.cooldown_active() {
        tracing::info!(trace_id = %plan.trace_id, "tier change skipped: cooldown active");
        return self.run_turn_inner(user_message).await;
    }

    // 5. Apply the model if it changed
    let new_model = self.tier_to_model(plan.tier);
    if new_model != self.current_model() {
        self.provider.set_model(&new_model).await?;
        self.record_swap();
    }

    // 6. Apply the reasoning effort if it changed
    if let Some(new_effort) = &plan.effort {
        if new_effort != self.current_effort() {
            self.provider.set_reasoning_effort(new_effort).await?;
        }
    }

    // 7. Persist trace
    self.persist_router_trace(&plan, &user_message);

    // 8. Run the turn
    self.run_turn_inner(user_message).await
}
```

### Provider whitelist (locked)

```rust
// crates/mona-acp/src/provider_whitelist.rs

pub enum SupportedProvider {
    Codex,    // OpenAI; default model: gpt-5.4 (Codex) or gpt-5.5
    Claude,   // Anthropic; default model: claude-sonnet-4-6 / claude-opus-5
    Minimax,  // MiniMax; default model: MiniMax-M3
}

impl SupportedProvider {
    pub fn parse(s: &str) -> Result<Self, ProviderError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "codex" | "openai" => Ok(Self::Codex),
            "claude" | "anthropic" => Ok(Self::Claude),
            "minimax" => Ok(Self::Minimax),
            other => Err(ProviderError::Unsupported {
                requested: other.to_string(),
                supported: &["codex", "claude", "minimax"],
            }),
        }
    }

    pub fn default_model(&self) -> &'static str {
        match self {
            Self::Codex => "gpt-5.5",
            Self::Claude => "claude-sonnet-4-6",
            Self::Minimax => "MiniMax-M3",
        }
    }

    pub fn default_effort(&self) -> &'static str {
        match self {
            Self::Codex => "high",
            Self::Claude => "high",
            Self::Minimax => "high",
        }
    }
}
```

`mona acp --provider codex|claude|minimax` accepts only these three. Anything else errors with:

```
Error: provider 'gemini' is not supported in mona Phase 2.
Supported providers: codex, claude, minimax.
See docs/PHASE-2-MONA-ACP.md for the full provider plan.
```

The full provider matrix (Phase 4) will add `openrouter`, `bedrock`, `copilot`, etc.

## File-by-file plan

### 1. Add `crates/mona-jev/`

- New crate, depends on `mona-provider-core`, `serde`, `tokio`, `reqwest`, `uuid`, `anyhow`.
- Re-implements the `JevClassifier` trait from Monitter's `src-tauri/src/model_router.rs:380`.
- `live.rs` calls the JEV HTTP endpoint with `JEV_API_KEY` read from Keychain (or env-var fallback for non-macOS dev).
- `safety.rs` implements the four hard gates: sensitive-prompt short-circuit, never-widen-permission, cooldown violation, confidence floor.
- ~600 lines of Rust, ~150 lines of tests.

### 2. Add `crates/mona-acp/`

- New crate, depends on `mona-jev`, `mona-app-core`, `mona-base`, `mona-provider-core`, `agent-client-protocol = "=0.10.4"` with the same features `jcode` already uses (`unstable_session_model`, `unstable_session_resume`, `unstable_session_usage`).
- `server.rs`: ACP stdio server loop, ~150 lines.
- `routes/*.rs`: one handler per ACP method, ~100 lines each = ~600 lines total.
- `policy.rs`: `JevRoutePolicy` enum + config loading, ~80 lines.
- `cooldown.rs`: per-session counter, ~50 lines.
- `bridge.rs`: adapter between ACP request stream and `mona-app-core`'s turn-execution, ~150 lines.
- `provider_whitelist.rs`: ~50 lines (see above).
- Tests: ~400 lines.
- Total: ~1,500 lines.

### 3. Add `crates/mona-acp` to `Cargo.toml` workspace

One new line in `[workspace.members]`.

### 4. Add `[[bin]]` for `mona-acp`

New binary target. The existing `mona` binary keeps its stripped CLI; the new `mona-acp` binary is the Phase 2 native replacement. Eventually Phase 4 removes the `mona` binary entirely, but for now both exist.

### 5. Wire into `Agent::run_turn`

Touch one function in `crates/mona-app-core/src/agent/turn_execution.rs`. The current `run_turn` is in the file we already read; we'll add a `run_turn_with_jev` wrapper.

### 6. Configure `agent-client-protocol` peer

`crates/mona-acp/Cargo.toml`:

```toml
[dependencies]
agent-client-protocol = { version = "=0.10.4",
                          features = ["unstable_session_model",
                                      "unstable_session_resume",
                                      "unstable_session_usage"] }
mona-app-core = { path = "../mona-app-core" }
mona-base = { path = "../mona-base" }
mona-provider-core = { path = "../mona-provider-core" }
mona-jev = { path = "../mona-jev" }
mona-message-types = { path = "../mona-message-types" }
anyhow = "1"
async-trait = "0.1"
futures = "0.3"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tokio-stream = "0.1"
tracing = "0.1"
uuid = { version = "1", features = ["v4"] }
```

### 7. Document in `docs/MONA-ACP-SERVER.md`

Add a section on the Phase 2 implementation status. The design doc already exists; this is just an update.

### 8. Tests

- `crates/mona-jev/tests/safety_gates_test.rs`: 5 tests covering each safety rule.
- `crates/mona-acp/tests/end_to_end.rs`: spawn `mona-acp`, send `initialize`, `session/new`, `session/prompt`, `session/set_model`, assert events.
- Manual smoke test against one provider (codex or claude).

### 9. Update `FUTURE-CLEANUPS.md`

Mark Phase 2 items as "in progress" and add a Phase 2 status section.

## Wire-protocol additions beyond standard ACP

Same as the design doc:

- `session/set_reasoning_effort` — Monitter extension for per-turn effort control.
- `session/jev_route` — advisory hint from client.
- `session/router_trace` — push event when a routing decision fires.

All three are negotiated in `initialize.capabilities`:
- `jev_routing: bool` — true when a JEV credential is configured AND at least one provider is authenticated.
- `reasoning_effort: bool` — always true in Phase 2.
- `unstable_session_model`, `unstable_session_resume`, `unstable_session_usage` — same as upstream.

## Failure modes (verified by named tests)

| Failure | Test name | Behavior |
|---|---|---|
| Provider string not in whitelist | `whitelist_rejects_unknown_provider` | Errors with supported list |
| Jev classifier unreachable | `clawback_uses_cached_plan_on_unavailable` | Uses last cached plan; emits trace with `trigger: Unavailable` |
| `set_model` returns Err from provider | `set_model_failure_returns_internal_error` | Re-attempts with `set_model_with_auth_refresh`; if still fails, surfaces as ACP `internal_error` |
| Cooldown violated | `cooldown_returns_throttled_with_retry_after` | ACP `request_throttled` with `{ retry_after_turns: N }` |
| Sensitive prompt | `sensitive_prompt_returns_permission_required` | ACP `permission_required` |
| Auth expired mid-session | `auth_expired_returns_unauthenticated` | Re-read auth from disk; if still expired, ACP `unauthenticated` and re-auth prompt |
| Protocol version mismatch | `protocol_version_negotiation_downgrades_safely` | Negotiate down to min(client, server) version |

## Effort estimate (revised)

| Workstream | Effort | Confidence |
|---|---|---|
| `crates/mona-jev/` (port `JevClassifier` trait from Monitter's `model_router.rs`) | 1 week | high |
| `crates/mona-acp/` (server loop, route handlers, capability negotiation) | 2 weeks | medium-high — depends on agent-client-protocol 0.10.4 server-side API surface |
| Provider whitelist + reject non-Phase-2 providers | 2 days | high |
| Cooldown + safety gates + trace persistence | 1 week | high |
| Wire into `Agent::run_turn` (or equivalent) | 1 week | medium — depends on call site compat |
| Update `docs/MONA-ACP-SERVER.md` to mark Phase 2 done | 1 day | high |
| Tests: unit, integration, end-to-end against a real provider | 1 week | high |
| **Total** | **~6–8 engineer-weeks** | |

## How Phase 2 connects back to Monitter

The design of `crates/mona-acp` is meant to be **identical** to upstream's `crates/jcode-harness-api-server` plus the per-turn Jev routing. This means:

1. **Monitter's existing ACP adapter** (`src-tauri/src/acp_transport.rs`, `acp_runtime.rs`, `acp_protocol.rs`, `acp_session_config.rs`, `acp_discovery.rs`) needs no changes. It already speaks the ACP wire protocol upstream uses.
2. **`acp_discovery.rs`** picks up `mona-acp` automatically because we use the same `agent-client-protocol = "0.10.4"` version and the same wire protocol shape. The "Per-turn Jev routing" badge appears automatically when `initialize.capabilities.jev_routing: true`.
3. **Mobile + share-web** drive `mona-acp` over ACP stdio. No new client code.
4. **Per-turn tier chip** in the Monitter UI lights up because the same `model_info` push event already exists; we add a `router_trace` push event for the Jev decision.

## Decision points

Three small calls I'd want your input on before starting:

1. **Default model per provider** — I proposed `gpt-5.5`, `claude-sonnet-4-6`, `MiniMax-M3`. Real or aspirational?
2. **Cooldown default** — 1 swap per 2 turns (N=2). Tighter (1 per 3 turns) costs latency, looser (1 per turn) wastes classification calls.
3. **Live Jev call or local-only** — Phase 2 can ship with the in-memory mock classifier; live HTTP calls to a Jev endpoint come in Phase 2.5. Faster to ship, but should we plan for live from day 1?

## Stop conditions

Stop and re-check with Alex if:

- The upstream `Provider::set_model` signature breaks in a way that breaks `set_model_with_auth_refresh`.
- Phase 2 hits >2 weeks of compile/iterate time without producing a buildable binary.
- A real provider in the whitelist (Codex, Claude, MiniMax) refuses `set_model` mid-session in a way that's not recoverable via `set_model_with_auth_refresh`.
- The JEV HTTP API contract changes (e.g. new auth scheme, new response shape).
