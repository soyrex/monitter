# Phase 3 status — auth loader shipped, agent-loop integration deferred

Phase 3 shipped in commit `2f4d504+1` on `feature/mona-acp-phase-2`. This document captures what landed, what was deferred, and why.

## What shipped in Phase 3

### Auth loader (`crates/mona-acp/src/auth.rs`, new, ~280 lines)

`AuthRegistry` reads `~/.mona/<provider>.json` at startup for each Phase 2 provider:

| File | Auth variant |
|---|---|
| `~/.mona/codex.json` | `OpenaiOauth` or `OpenaiApiKey` |
| `~/.mona/claude.json` | `AnthropicOauth` or `AnthropicApiKey` |
| `~/.mona/minimax.json` | `MinimaxApiKey` |

Each auth record is JSON-tagged:

```json
{ "kind": "openai_api_key", "api_key": "sk-..." }
{ "kind": "openai_oauth", "access_token": "...", "refresh_token": "...", "expires_at_ms": 0 }
{ "kind": "anthropic_api_key", "api_key": "sk-ant-..." }
{ "kind": "minimax_api_key", "api_key": "...", "api_base": "https://api.minimax.io/v1" }
```

If no file exists, the loader falls back to env vars: `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `MINIMAX_API_KEY`. If neither file nor env var exists, the provider is logged as "not configured" and `session/auth` reports it accordingly. The loader is permissive — missing auth is not an error.

### New `session/auth` RPC method

Returns the auth state for a session's provider:

```json
{ "configured": true,  "summary": "OpenAI API key (sk-t…cdef)", "provider": "codex", "phase": "3" }
{ "configured": false, "summary": null, "provider": "claude", "phase": "3", "hint": "place credentials at ~/.mona/claude.json or set the matching env var" }
```

Credentials are masked in the summary (first 4 + last 4 chars visible). Safe to log.

### Updated `initialize` response

Now includes:
- `configuredProviders`: array of providers that have auth loaded at startup.
- `agentCapabilities.extensions.monitter.auth_loader`: true (advertises Phase 3 capability).
- `phase`: still implicit; surfaced via the `agentCapabilities.extensions.monitter` keys.

### Test coverage

- 24 lib tests pass (was 17 before Phase 3, +7 new for auth/initialize).
- 7 e2e tests pass (was 5 before Phase 3, +2 new for the auth loader).

```
test auth_loader_reports_unconfigured_provider ... ok
test auth_loader_picks_up_api_key_file ... ok
```

## What was deliberately deferred

### `Agent::run_turn()` integration

The session/prompt handler still returns a routing decision as its response — it does NOT drive a real `Agent::run_turn` against a configured provider.

The honest reason: **`Agent::run_turn` lives in `crates/mona-app-core`, which depends on ~30 upstream crates including the tool runtime, MCP, prompt rendering, session persistence, and the full provider stack.** Wiring `mona-acp` to that surface would require:

1. Adding `mona-app-core`, `mona-base`, `mona-provider-openai-runtime`, `mona-provider-anthropic-runtime`, `mona-tool-core`, `mona-storage`, `mona-session-types`, `mona-protocol`, `mona-message-types`, etc. as dependencies of `mona-acp`. Estimated cold-build cost: +5–10 minutes. Estimated binary size: +40–60 MB (from 2.2 MB to ~50 MB).
2. Building an `Agent` per session from a `Provider` constructed from `AuthRegistry`. The `Provider` trait is already imported via `mona_provider_core::Provider`; constructing concrete instances for Codex/Claude/MiniMax requires the upstream `OpenAiProvider`, `AnthropicProvider`, etc. — each of which has its own configuration shape.
3. Mapping upstream's `ServerEvent` types (`text_delta`, `tool_call_started`, etc.) to ACP wire events. That's a per-event-type mapping with ~15+ event variants.
4. Mapping the upstream daemon's streaming RPC protocol to ACP's request/response protocol. The upstream `run_once_streaming_mpsc` takes an `mpsc::UnboundedSender<ServerEvent>`; ACP expects JSON-RPC responses + push notifications on stdout. This translation is the bulk of the work.

Estimated effort: 3–5 engineer-days for a competent Rust developer. **That's a separate, focused project**, not a "next 30 minutes" task.

### Live Jev HTTP classifier

Still Phase 2.5-deferred per Alex's earlier decision. `RuleBasedClassifier` continues to be the default.

### Provider construction from `Auth`

We have `AuthRegistry` (knows about credentials) but we don't yet construct `Arc<dyn Provider>` instances from them. Phase 3.5 (next session) will do that — once providers exist, the auth loader becomes the only thing standing between a session and a real `Agent::run_turn` call.

## What we know works end-to-end

```
session/new  →  creates session, defaults model/effort
session/auth →  reports configured/!configured per provider
session/list →  shows active sessions
session/cancel →  removes session
session/set_model →  validates + records
session/set_reasoning_effort →  records
session/prompt →  runs per-turn Jev classifier + safety gates + trace
                persistence + session update; returns routing decision
                (no actual agent loop yet)
```

What this means practically: **Monitter desktop can already drive `mona-acp` for everything except actual model responses.** A Monitter user with valid `codex.json` credentials can see the routing work end-to-end — Jev classifies, safety gates filter, tier changes apply, traces persist. They just don't see text come back from a model yet. That's the next milestone.

## Recommended next session (Phase 3.5)

Pick one of these in priority order:

1. **Provider construction** — implement `Provider::from(Auth)` for the three Phase 2 providers. This unblocks the agent-loop integration. Estimated 1 engineer-day.
2. **Agent::run_turn integration** — wire `mona-acp` to drive `Agent::run_turn_streaming_mpsc` for each session. Map `ServerEvent` to ACP wire events. Estimated 3–5 engineer-days.
3. **End-to-end smoke test against a real provider** — with option 1 in place, run `mona-acp` against `OPENAI_API_KEY` and confirm a real model responds. Estimated 0.5 day.

I'd recommend doing **option 1** first as a standalone commit (small, reviewable, unblocks everything else), then **option 3** as a smoke test (proves the auth + provider chain actually works), then **option 2** as the full wiring.

## Branch status

```
feature/mona-acp-phase-2 (5 commits)
├── <this commit> Phase 3: auth loader + session/auth RPC method
├── 2f4d504 Phase 2.5: per-turn Jev routing hook fires inside session/prompt
├── 814c3df Add crates/mona-acp/ — Phase 2 ACP stdio server
├── 213bfc6 Add crates/mona-jev/ — Jev classifier foundation
└── (master: e589cbe upstream v0.86.0)
```

Workflow files (`.github/workflows/*.yml`) excluded pending OAuth workflow-scope token.
