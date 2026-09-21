# Goal: `mona` — full stack with mobile + share-web

Status: **Active**.
Created: 2026-09-20.
Owner: Alex Holt (Holt Seafood Company Pty Ltd).
Decisions in this conversation: scope = full Phase 1 + Phase 2; deliverable = full ACP server integration; location = isolated directory.

## Outcome

`mona` — a forked, renamed, Monitter-owned derivative of `jcode v0.86.0` — runs as a peer harness in Monitter desktop, mobile, and share-web, with per-turn Jev routing on every turn.

## Phase 1 — fork + rename + per-turn Jev routing

| Acceptance criterion | Evidence |
|---|---|
| `github.com/soyrex/mona` exists as a public repo with verbatim `LICENSE` (MIT from `jcode`) + `MONA_NOTICE.md` | Repo on github.com |
| `cargo build --release` from a clean checkout produces a `mona` binary | Build log |
| `mona serve` listens on `~/.mona/api.sock` and accepts JSON requests | Smoke-test output |
| `mona-sdk` Rust crate exposes `Client::open_session`, `set_model`, `set_reasoning_effort`, `send_message`, `cancel`, `list_models`, `attach_session`, `fork_session` | `cargo doc` |
| Jev classifier is embedded inside `mona`'s `Agent::run_turn` — every user turn triggers `JevClassifier::classify`, which calls `set_model_with_auth_refresh` if the decision differs from the current state | Per-turn trace JSON in `~/.mona/router-traces/` |
| Cooldown, confidence floor, never-widen-permission-tier, sensitive-prompt gate all enforced | Unit tests in `monacrate-jev` |
| Per-turn traces persisted to `~/.mona/router-traces/` with the same shape as Monitter's `.monitter/router-traces/` so the Monitter UI can render either | Two sample traces |
| Monitter desktop can launch a `mona` task via `mona-sdk` and the per-turn tier chip renders in the run detail UI | Screenshot |
| Existing Codex/Claude/OpenCode adapters are untouched | `git diff main` |

## Phase 2 — `mona-acp` stdio server

**Phase 2 provider set (locked 2026-09-20)**: OpenAI (`codex`, `--provider codex`), Anthropic (`claude`, `--provider claude`), MiniMax (`minimax`, `--provider minimax`). All three already wired in upstream and inherited verbatim by the rename. Build a `--provider {codex,claude,minimax}` whitelist in the Phase 2 CLI; reject others with a clear message pointing at the full Phase 4 supported-provider matrix.

| Acceptance criterion | Evidence |
|---|---|
| `crates/mona-acp/` exists in the fork with `server.rs`, `routes/{initialize,authenticate,session,model,effort,trace}.rs`, `policy.rs`, `cooldown.rs`, `bridge.rs` | `crates/mona-acp/src/` tree |
| `mona-acp` binary accepts ACP stdio and responds to `initialize` with `capabilities: { session_model: true, session_resume: true, session_usage: true, jev_routing: true, reasoning_effort: true }` | Transcript from `tests/end_to_end.rs` |
| `mona-acp` handles `session/new`, `session/prompt`, `session/cancel`, `session/resume`, `session/set_model`, `session/set_reasoning_effort` | Same transcript |
| `mona-acp --provider` accepts exactly `codex`, `claude`, `minimax`; rejects everything else with a message pointing at the full provider matrix | Help output + test |
| Per-turn Jev routing fires inside `Agent::run_turn` (or equivalent hook point in `mona_app_core::agent::turn_execution`) before each `provider.complete()` call | Trace JSON in `~/.mona/router-traces/{session}-{turn}.json` |
| Cooldown (≤1 swap per N turns, default N=2), confidence floor (≥0.5), never-widen-permission-tier, sensitive-prompt gate all enforced | Unit tests in `monacrate-jev` |
| Monitter's `acp_discovery.rs` auto-detects `mona-acp` on `$PATH` and surfaces the "Per-turn Jev routing" badge | Discovery log |
| Mobile and share-web clients can drive a `mona-acp` session over the existing ACP transport — no new transport code on the client side | iOS + share-web screenshots |
| Failure modes (Jev unreachable, provider set_model fail, unsupported effort, cooldown, sensitive prompt, auth expired, protocol version mismatch) all behave per the spec table in `docs/MONA-ACP-SERVER.md` § Failure modes | Named tests in `crates/mona-acp/tests/end_to_end.rs` |
| `agent-client-protocol = "=0.10.4"` pinned with the same `unstable_session_model`, `unstable_session_resume`, `unstable_session_usage` features | `Cargo.lock` |
| `MONA_NOTICE.md` exists at fork root referencing upstream `jcode` MIT license | File on disk |

## Naming (locked 2026-09-20)

| Concept | Name |
|---|---|
| Binary | `mona` |
| Crate root | `mona` |
| Workspace crates | `mona-*` |
| Home dir | `~/.mona/` |
| Auth files | `~/.mona/openai-auth.json` etc. |
| Sockets | `~/.mona/mona.sock`, `~/.mona/api.sock` |
| Env vars | `MONA_SOCKET`, `MONA_API_SOCKET`, `MONA_ALLOW_CODEX_LEGACY_AUTH` |
| Provider strings | Unchanged (`openai`, `claude`, `gemini`, ...) |
| Wire protocol | Unchanged (ACP `SetModel` etc.) |
| GitHub | `github.com/soyrex/mona` |
| Upstream ref | `github.com/1jehuang/jcode` |
| License | MIT from `jcode`, preserved verbatim |
| Self-dev gate | `mona-selfdev-types` looking for `crates/mona-desktop-ui/` |

## Working location

`/Users/alex/code/monitter-jcode-audit/` — fully isolated from `/Users/alex/code/monitter/`. Monitter main checkout untouched. `docs/MONA-ACP-SERVER.md` (525 lines, 23 KB) lives in the main checkout as the design of record.

## Effort estimate

- Phase 1: ~5 engineer-weeks
- Phase 2: ~5–7 engineer-weeks
- Combined: ~10–12 engineer-weeks

## Stop conditions

Stop and re-check with Alex if:

- The upstream `jcode` license changes (currently MIT).
- The upstream `Provider` trait breaks in a way that invalidates the `set_model_with_auth_refresh` hook.
- The ACP server-side API surface of `agent-client-protocol 0.10.4` proves too restrictive to express the Monitter extensions cleanly.
- A regulatory or compliance issue surfaces around the AI-assisted self-modification pattern (the fork has a `selfdev` profile; the gate is path-based and naturally inert, but the contract is worth re-reading before merge).

## Reference documents

- `docs/MONA-ACP-SERVER.md` — full design (525 lines)
- `crates/jcode-provider-core/src/lib.rs:77–283` — `Provider` trait (upstream reference)
- `crates/jcode-harness-api-server/src/translate.rs:892–930` — wire handlers for `set_model` and `set_reasoning_effort` (upstream reference)
- `crates/jcode-sdk/src/client.rs:1141–1158` — Rust SDK surface (upstream reference)
- `crates/jcode-provider-grok-build-runtime/src/lib.rs:391–473` — inverse pattern (fork as ACP *client*) we flip to server
- `crates/jcode-selfdev-types/src/desktop.rs:8–19` — `desktop_repo_root()` becomes `mona-desktop-ui` lookup after rename
- `src-tauri/src/acp_transport.rs:1–25` — Monitter-side ACP adapter, harness-agnostic by design
- `src-tauri/src/model_router.rs:380–773` — `JevClassifier` trait and `LiveJevClassifier` impl (Monitter reference)
- `src-tauri/src/lib.rs:4096–4121` — `plan_jev_route` Tauri command (Monitter reference)
- `LICENSE` (upstream) — MIT, Copyright (c) 2025 Jeremy Huang
