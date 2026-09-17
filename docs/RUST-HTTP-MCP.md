# Rust collaboration MCP

The built-in collaboration server now runs inside the Rust desktop process. Local
harnesses connect to its authenticated loopback `/mcp` endpoint. Remote harnesses
connect to a remote-loopback port forwarded over an owned SSH reverse tunnel to
the same broker. No Python MCP helper is launched, extracted, or uploaded.

The collaboration tool schemas and initialization instructions live in
`src-tauri/src/collaboration_mcp.rs`. Native allowlists use the same Rust tool-name
catalogue. The server supports JSON-only Streamable HTTP, protocol negotiation,
authenticated discovery, bounded requests/concurrency, and revoked-grant rejection.
It does not provide an SSE stream or issue MCP session IDs.
The supported protocol versions are `2025-03-26` and `2025-06-18`; newer clients
negotiate a supported version. Requests require `Content-Length` (chunked uploads
are not supported), are limited to 128 KiB, and must accept JSON and event-stream
responses. Headers are limited to 16 KiB and concurrent requests to sixteen.

## Ownership and compatibility

- Grants remain scoped to the owning task and process. Resident sessions retain
  their grant between turns; actual tool calls still require a running task and
  enabled collaboration. Process teardown revokes the grant.
- SSH startup, cancellation, early failure, and normal teardown clean up only the
  owned tunnel. The endpoint passed to the remote harness uses the forwarded
  remote port. Tokens travel through private environment/session channels, not
  command arguments or saved settings.
- Codex uses an HTTP URL and `bearer_token_env_var`. Claude uses an invocation-only
  HTTP configuration with environment-expanded authorization. OpenCode uses its
  installed 1.18 direct `mcp.monitter` remote-server shape, merging existing inline
  settings. ACP uses an HTTP session-server definition and requires the agent to
  advertise HTTP MCP support; unsupported agents fail visibly before a prompt.
- Existing native permissions and other user-configured MCP servers are retained.
  This does not add managed third-party MCP support over SSH or remote Claude
  interactive-session support.

## Scope of Python removal

This removes the collaboration MCP relay only. Python-based SSH process
supervision, the Hermes bridge, and unrelated helpers are unchanged. It eliminates
Python subprocesses from this MCP path but does not establish the cause of every
reported `python3` crash or guarantee that unrelated crashes disappear.

## Repeatable checks

### Verification on 2026-09-16

- Final serial Rust library suite: **351 passed, zero failed, seven opt-in tests
  ignored**. This includes actual-broker two-turn resident Codex fixtures, ACP
  capability/recovery fixtures, HTTP validation/auth/revocation, and tunnel
  startup/cancellation/early-failure cleanup.
- Installed Codex 0.154.0: the original feature opt-in probe passed against the real Rust broker
  and discovered the then-current eight Monitter tools. It used temporary settings
  and an ephemeral task, without a model turn or changes to the user's CLI config.
  Main subsequently added three shared-skill tools; the merged catalogue has eleven.
- Real SSH alias `mira`: the opt-in tunnel test passed, including an MCP tool
  request from the remote host, rejection after grant revocation, and tunnel
  cleanup. No helper or service was installed on Mira.
- Node app-server fixture and controller protocol checks passed; all three ACP
  remote-supervisor checks passed. The direct Vite production build passed.
- The macOS arm64 debug app was packaged, ad-hoc signed, and passed strict deep
  signature verification. Artifact: `src-tauri/target/debug/bundle/macos/Monitter.app`.
  It was not installed or launched; no live LAN assets were published. This is a
  development build, not a notarized release. Two existing non-test unused-code
  warnings remain in `runner.rs`.
- Claude 2.1.270 and OpenCode 1.18.31 configuration formats were checked; their
  adapter/configuration tests passed. No real Claude/OpenCode model delegation
  or live ACP model turn is claimed for this migration.

Early parallel suite runs exposed stale fixture cleanup assumptions and transient
process-start/PTY timing failures. The fixtures now advertise HTTP support and
terminate owned idle residents explicitly. No unrelated terminal/OpenCode
production behavior was changed; the final serial run passed in full.

### Commands

Run from the feature worktree. Avoid `npm run build` during private verification:
that script also publishes LAN assets for the running app.

```sh
npm exec vite build
node scripts/app-server-protocol-test.mjs
node scripts/controller-protocol-test.mjs
python3 scripts/test-acp-remote.py
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
  cargo test --manifest-path src-tauri/Cargo.toml --lib -j 4 -- --test-threads=1
```

Two ignored checks require explicit local choices. The installed-Codex probe uses
temporary settings, initializes the actual Rust broker, and does not request a
model turn. The SSH check uses an existing trusted login and remote `curl`; it
verifies an MCP request, grant revocation, and tunnel cleanup without installing
anything remotely.

```sh
MONITTER_TEST_CODEX=/absolute/path/to/codex \
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
  cargo test --manifest-path src-tauri/Cargo.toml --lib \
  installed_codex_app_server_loads_http_monitter_mcp -- --ignored --nocapture
MONITTER_TEST_SSH_HOST=trusted-host-alias \
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
  cargo test --manifest-path src-tauri/Cargo.toml --lib \
  http_mcp_over_real_ssh_tunnel -- --ignored --nocapture
```

Private application packaging uses already-built frontend assets and bypasses the
publishing hook. It does not install or start the resulting app:

```sh
CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 \
  npm run tauri -- build --debug --bundles app --no-sign \
  --config '{"build":{"beforeBuildCommand":""}}'
codesign --force --deep --sign - src-tauri/target/debug/bundle/macos/Monitter.app
codesign --verify --deep --strict src-tauri/target/debug/bundle/macos/Monitter.app
```
