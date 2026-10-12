# Monitter agent guide

## Desktop and mobile build boundary

GPUI is the installed desktop, including `/Applications/Monitter.app`.
Desktop changes, builds and installs use this checkout and `npm run build:gpui`.
Tauri/Svelte is for mobile web and iOS/Android; never use the legacy `build:mac`
or `install:mac` scripts to update the installed desktop. Preserve the live
backend and its agent sessions when updating the GUI. Alex's explicit desktop
reinstall authorization permits replacing the installed GPUI bundle with a
verified build and a rollback copy, superseding the experimental restriction below.

## Experimental GPUI fork

This checkout is the `experiment/gpui-desktop` fork from `557bb5e`. Alex has
authorized migrating the full desktop UI to GPUI while leaving mobile and shared
web clients unchanged. For this fork, `desktop-gpui/` uses Rust and pinned GPUI
Kit; this explicitly supersedes the Svelte desktop stack requirement below.
Retain the Rust authority and the existing mobile protocol. Track incomplete
features in `desktop-gpui/parity.json`; a working chat shell is not full parity.

Match outcomes and visual design, not the Tauri/Svelte implementation path.
Prefer GPUI retained entities, native focus/actions/input, virtualized lists,
typed Rust state, and existing GPUI docking/resizing primitives where they fit.
Keep backend security/execution authoritative. The JSON command adapter is an
incremental migration seam, not a mandate to preserve renderer IPC or polling.
Evaluate native alternatives before porting DOM/workaround behavior literally.

Use `npm run check:gpui`, `npm run build:gpui`, and `npm run dev:gpui` for the
native GPUI executable. `build:mac` remains the legacy Tauri baseline, even though
its packaging identity is isolated. Never call it proof of a GPUI build.
The experimental profile is `com.monitter.gpui.experimental`. Do not copy or open
the production profile, overwrite `/Applications/Monitter.app`, restart production,
or deploy the unchanged production relay infrastructure. Legacy smoke scripts
must be audited for production paths/ports before running them.

Track cache growth with `node scripts/gpui-disk.mjs`. Native commands share
`.cargo-target/gpui`; packaging records before/after usage under ignored
`artifacts/gpui/disk-usage.jsonl` and stops below 6 GiB free for warm debug
rebuilds, or 10 GiB for cold/release builds. Reclaim only idle,
regenerable fork-owned caches after checking active compiler processes. Preserve
production/shared caches, running app bundles, profiles, credentials and backups.

Monitter is a Tauri desktop workspace for real CLI and ACP agents. The desktop runs the agents locally or over SSH; mobile and shared browser clients connect to that desktop. macOS is the primary tested desktop platform. Do not edit the unrelated CME checkout.

## Project map

- `src/routes/` contains the SvelteKit entry points for the desktop, mobile web view, and shared view.
- `src/lib/components/` contains Svelte UI components; `src/lib/` also holds UI state, the native/LAN bridge, and canonical TypeScript shapes in `types.ts`.
- `src-tauri/src/` contains the Rust authority for persistence, Tauri commands, harness execution, ACP, collaboration, SSH/LAN access, and permissions. `src-tauri/permissions/` contains Tauri capability grants.
- `mobile-ios/` and `mobile-android/` are paired controllers. `share-web/` is the shared browser client. `relay-cloudflare/` and `deploy/relay/` contain optional relay code and deployment material.
- `docs/CONTRACT.md` defines the frontend/backend protocol. `design/reference.html` is the visual reference; `design/` holds supporting styles. `scripts/` contains build and focused verification tools.

## Code choices and boundaries

- Use **Tauri 2 + Rust** for native services, process and filesystem access, persistence, and security decisions. Use **Svelte 5 + SvelteKit + TypeScript** for the UI, with Vite/npm for development. Follow existing patterns before introducing another framework or dependency.
- Read `docs/CONTRACT.md` and `src/lib/types.ts` before changing data exchanged across the frontend/backend boundary. Keep Rust serde fields and TypeScript fields aligned, use camelCase JSON, Unix millisecond timestamps, and UUID string IDs as the contract specifies. Update the contract when behavior or commands change.
- The Rust backend is authoritative for task and agent state. Use the existing bridge and scoped Tauri/LAN permissions; do not move privileged operations into the renderer or widen remote access for UI convenience. Keep errors visible.
- Preserve users' CLI authentication, configuration, native sessions, transcripts, and data. Use real installed harnesses, starting with Codex; never fabricate working agents, conversations, progress, usage, host connections, or model replies.
- Match `design/reference.html` where applicable and retain configurable accent plus light and dark themes. Keep mobile and shared-client permissions appropriate to their roles.
- Do not overwrite unrelated edits. Inspect Git status before editing; use an isolated worktree for substantial or risky changes when the main checkout is dirty.

## Delegation and model cost

- Delegate independent, bounded work when it saves time and the parent can make useful progress alongside it. Handle small or sequential work directly. **For every subagent, choose the least expensive available model and route that can reliably do the assigned task**; raise capability only when complexity or observed results justify it.
- For built-in Codex agents, prefer Luna for extraction, codebase lookups, and focused checks; Terra for implementation and harder investigation; Sol for complex architecture or synthesis. These are starting points, not fixed requirements. Use a stronger model when the lower-cost choice has failed or would create material rework.
- Monitter's collaboration MCP may expose agents from other providers. When available and authorized, consider those agents alongside built-in subagents. Compare likely total cost, capability, latency, tool access, and data scope; do not assume a provider is cheaper based only on its name or a stale price. Do not install, subscribe, buy credits, or change provider settings merely to delegate.
- Give each delegate a self-contained brief with the objective, relevant paths/evidence, authorization, file or responsibility ownership, and expected output. Prefer `fork_turns="none"` when sufficient. Tell editing agents that others share the checkout and that they must preserve others' changes. Reuse an existing agent for related follow-ups and avoid repeating completed investigation.
- Keep delegation permissions scoped. Review results and verify important claims before integrating them. A successful ACP handshake alone does not prove an authenticated, working model turn.

## Working and verification

- Use `npm ci` for a clean dependency install. `npm run check` checks Svelte/TypeScript; `npm run build` builds the web assets; `cargo test --manifest-path src-tauri/Cargo.toml` runs Rust tests. Choose focused tests from `scripts/` for the changed behavior, then broaden only for a concrete remaining risk.
- `npm run tauri dev` runs the desktop app. `npm run dev` is a browser design preview and does not run native agents. For the installed macOS app, `npm run dev:app-ui` serves the hot UI on the dedicated bridge port; backend edits require a native rebuild.
- Do not purchase or provision paid services. A specific transaction requires Alex's explicit approval of vendor/account, total price and currency, term, and recurring charges before any charge or commitment; research or setup approval is not purchase approval.

## Experimental preview lifecycle authorization

Alex has authorized this agent to own experimental preview launches and restarts for development and verification. This supersedes the earlier requirement to ask before restarting the preview. Verify the exact experimental executable before stopping it. Production Monitter, mobile/shared clients, profiles, and unrelated processes remain protected.
