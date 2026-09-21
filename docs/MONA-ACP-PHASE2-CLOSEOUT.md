# Mona ACP Phase 2 desktop verification

Phase 2 is testable in Monitter without spending provider credits. The local
smoke starts the real `mona-acp` executable through Monitter's ACP transport,
uses an isolated empty `MONA_HOME`, and verifies initialize, session creation,
routing trace delivery, and the expected unauthenticated prompt result.

## Build and install Mona ACP

From the Mona checkout:

```sh
cargo build --release --bin mona-acp
install -d "$HOME/.local/bin"
install -m 0755 target/release/mona-acp "$HOME/.local/bin/mona-acp"
```

Monitter's reviewed ACP catalog looks for `mona-acp` on `PATH` and in
`~/.local/bin`. In the desktop app, open the agent settings, choose ACP, refresh
discovery, select **mona-acp (Monitter harness)**, and use **Verify connection**.
Verification proves protocol compatibility and reports negotiated Jev routing,
auth-loader, and reasoning-effort capabilities. It does not prove provider
authentication or make a model request.

## No-network verification

```sh
cd /Users/alex/code/mona-acp-milestone-a
cargo test -p mona-acp --lib
cargo test -p mona-acp --test end_to_end
cargo build --bin mona-acp

cd /Users/alex/code/monitter-mona-phase2-closeout
npm run check
node scripts/run-activity-mona-router-trace-isolated.mjs
RUN_MONITTER_MONA_SYNTHETIC_SMOKE=1 \
  MONA_ACP_BIN=/Users/alex/code/mona-acp-milestone-a/target/debug/mona-acp \
  cargo test --manifest-path src-tauri/Cargo.toml \
  acp_mona_smoke::mona_acp_synthetic_router_trace_smoke --lib -- --nocapture
```

The synthetic smoke must run with an empty isolated Mona home. A routing trace
is expected, followed by a truthful unauthenticated error; any provider request
would be a test failure.

## Desktop build and installation

```sh
npm run build:mac
npm run install:mac -- --defer-restart
```

The installer verifies the signed bundle and preserves the previous installed
app as a timestamped backup. `--defer-restart` leaves an already-running
Monitter process untouched; restart it when ready to load the installed build.

## Paid live-provider boundary

The existing MiniMax smoke remains opt-in behind
`RUN_MONITTER_MINIMAX_SMOKE=1` and requires a provider credential. It is not
part of Phase 2's no-network acceptance run. Do not enable it without explicit
approval for the specific paid provider turn.
