# Phase 3.5 status — provider construction stub shipped; real wiring deferred

Phase 3.5 ships in commit `<pending>` on `feature/mona-acp-phase-2`. This document captures what landed (a stub foundation), what was deferred (real constructor wiring), and why.

## What shipped in Phase 3.5

### `ProviderHandle` + `AuthStubProvider` (`crates/mona-acp/src/provider.rs`, new, ~280 lines)

`session/new` now constructs a `ProviderHandle` per session. The handle wraps:

- `Arc<dyn Provider>` — currently an `AuthStubProvider` that implements the upstream `Provider` trait surface but refuses to drive a real model turn (returns a synthetic `StreamEvent::Error` with `phase3.5_stub_no_real_provider`).
- `Option<Auth>` — `Some` when auth was configured at session/new time, `None` when it wasn't.
- `SupportedProvider` — the whitelist-validated provider kind.

`build_provider_for_session(provider_str, &auth_registry)` is the single chokepoint for translating `AuthRegistry` → `ProviderHandle`. Phase 4 swaps the stub for the real `OpenAIProvider::new(CodexCredentials)` / `AnthropicProvider::new()` / `OpenRouterProvider::new_named_openai_compatible(MINIMAX_PROFILE)` constructors without changing the signature.

### New `session/new` response field: `providerName`

```json
{
  "sessionId": "d794...",
  "provider": "codex",
  "model": "gpt-5.5",
  "effort": "high",
  "providerName": "codex-stub",   // ← Phase 3.5
  "monitterPhase": "3.5",
  "jev_routing": true
}
```

`providerName` is the stub's stable identifier (`codex-stub`, `claude-stub`, `minimax-stub`). It surfaces as `null` when `session/new` was called without auth — sessions in that state are useful for read-only surfaces (`session/list`, `session/cancel`) but cannot drive a model turn until credentials are added.

### Missing-auth is no longer an error

Phase 3 originally returned `-32602 "no configured auth"` from `session/new` when credentials weren't present. Phase 3.5 relaxes this: `session/new` always succeeds and attaches a placeholder handle. Clients discover "this session has no auth" via `providerName: null` in the response or `session/auth`'s `configured: false`. This keeps the rest of the session lifecycle (list, cancel, set_model, prompt routing) testable end-to-end without a credential on every test.

### New e2e test: `session_new_reports_provider_name_when_auth_configured`

Verifies both branches:
- **Case A:** with `~/.mona/codex.json` present, `session/new` returns `providerName: "codex-stub"`.
- **Case B:** with no auth files, `session/new` returns `providerName: null`, and `session/auth` on the resulting session reports `configured: false` with the `~/.mona/codex.json` hint.

### Test coverage

- 33 lib tests pass (was 31 before Phase 3.5, +2 for `AuthStubProvider` round-trip).
- 8 e2e tests pass (was 7 before Phase 3.5, +1 for `providerName`).

```
test provider::tests::builds_handle_when_auth_configured ... ok
test provider::tests::builds_placeholder_handle_when_no_auth ... ok
test provider::tests::stub_provider_reports_kind_in_name ... ok
test provider::tests::stub_provider_listed_model_is_default_for_kind ... ok
test session::tests::new_session_attaches_provider_handle_when_auth_configured ... ok
test session::tests::new_session_creates_placeholder_handle_when_auth_missing ... ok
test session::tests::session_info_provider_name_null_when_unconfigured ... ok
test session::tests::session_info_provider_name_some_when_configured ... ok
test session_new_reports_provider_name_when_auth_configured ... ok
```

## What was deliberately deferred

### Real provider construction

`build_provider_for_session` still returns a stub. The real constructors pulled in:

- `mona-provider-openai-runtime` (~25 transitive deps; OAuth flow, Codex credentials struct, MAC-path env var handling).
- `mona-provider-anthropic-runtime` (~15 transitive deps; OAuth flow + API-key resolution).
- `mona-provider-openrouter-runtime` for MiniMax (openrouter cache namespace + profile key).
- `mona-base::auth::*` for the credential resolution helpers that upstream uses — coupled to daemon/macOS Keychain code we don't need.

The decision: **stub now, swap one function in Phase 4**. The `Provider` trait is already wired; `ProviderHandle`'s public surface is fixed; only the body of `build_provider_for_session` needs to change. Estimated 1–2 engineer-days for a focused Phase 4 commit.

### `Agent::run_turn` integration

Even with real provider construction, `session/prompt` still returns the Phase 2.5 routing decision as its response — it does NOT drive an `Agent::run_turn`. Per Phase 3's status doc, that's a 3–5 engineer-day project on its own.

## What we know works end-to-end (after Phase 3.5)

```
session/new        →  creates session, attaches ProviderHandle (stub or placeholder)
                     response now includes providerName field
session/auth       →  reports configured/!configured per session's provider
session/list       →  shows active sessions
session/cancel     →  removes session
session/set_model  →  validates + records
session/set_reasoning_effort → records
session/prompt     →  runs per-turn Jev classifier + safety gates + trace
                     persistence + session update; returns routing decision
                     (no actual agent loop yet — stub refuses with
                      phase3.5_stub_no_real_provider error)
```

What this means practically: **the auth + provider-handle plumbing is wired and tested**, so the next phase is purely "swap the stub body for real constructors and wire `Agent::run_turn`". No ACP-level work required.

## Recommended next session (Phase 4)

1. **Real provider construction** — implement `build_provider_for_session` against the upstream `OpenAIProvider`, `AnthropicProvider`, and OpenRouter-backed MiniMax constructors. Same signature; only the body changes. Estimated 1–2 engineer-days.
2. **End-to-end smoke test** — run `mona-acp` against `OPENAI_API_KEY` (or `~/.mona/codex.json`) and confirm a real model responds. Estimated 0.5 day.
3. **`Agent::run_turn` integration** — wire `mona-acp` to drive `Agent::run_turn_streaming_mpsc`. Map `ServerEvent` → ACP wire events. Estimated 3–5 engineer-days.

## Branch status

```
feature/mona-acp-phase-2 (6 commits)
├── <pending> Phase 3.5: provider construction stub + providerName field
├── 3d05f86 Phase 3: auth loader + session/auth RPC method
├── 2f4d504 Phase 2.5: per-turn Jev routing hook fires inside session/prompt
├── 814c3df Add crates/mona-acp/ — Phase 2 ACP stdio server
├── 213bfc6 Add crates/mona-jev/ — Jev classifier foundation
└── (master: e589cbe upstream v0.86.0)
```

Workflow files (`.github/workflows/*.yml`) excluded pending OAuth workflow-scope token.
