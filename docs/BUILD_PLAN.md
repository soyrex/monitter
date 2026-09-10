# Monitter: first macOS build

Working product name: Monitter. This is a new app informed by Orbit, not a fork of Orbit.

## Acceptance

- Installable native Tauri application that launches on Alex's Apple silicon Mac.
- The supplied warm paper/dark interface: nested agent/task sidebar, overview, conversation,
  right-side timeline and delegated task list, editable agents, channels and host settings.
- Configurable persistent accent, light/dark/system theme, readable text and keyboard-accessible controls.
- Multiple persistent agents and tasks, each bound to a harness, model, local/SSH host and folder.
- Working local Codex start, output streaming, resume/context continuity, cancellation and error display.
- Working system-SSH transport, connection probe, configurable CLI paths, folder and session-ID resume.
- Real local and SSH smoke tests when a user-approved test host is available.
- Configuration/history survive application restart. No mock results in the application.
- Per-task real run details; explicit delegation creates linked tasks. Channels route only to chosen agents.
- Native session ID and copyable terminal resume command; no claim of live Desktop attachment.

## Validation

Frontend type/build checks, native unit tests for command safety/event parsing/persistence/cancellation,
browser interactions and screenshots, native launch, live Codex first+follow-up turn, SSH first+follow-up
turn where available. Exact evidence and known limitations are recorded in VALIDATION.md.

## References

- User design: design/reference.html (provided HTML package)
- Orbit CLI adapters: https://github.com/xinnaider/orbit/blob/master/tauri/src/services/spawn_manager.rs
- Codex CLI: https://learn.chatgpt.com/docs/non-interactive-mode
- Tauri: https://v2.tauri.app/
- Prior shared-runtime proof: ../codex-desktop-companion-proof/REPORT.md

## Current work

The first macOS app is built, signed, installed and running. Frontend and native tests plus real
local/Mira Codex continuity and cancellation pass. Native message, SSH configuration, accent and
restart checks also pass. The integrated window header, bounded pane scrolling, lowercase wordmark
and 80–200% scale slider were built and installed with byte-identical saved state after restart.

The latest installed batch adds toggle switches, Cmd-K/Cmd-P, archive/delete controls, Codex goal
lookup, computer activity panel, and Claude/OpenCode/Hermes adapters. Automated verification covers
code and protocol fixtures. Native Cmd-K/Cmd-P were verified after installing this batch; live
account checks for the new harnesses remain. See VALIDATION.md for evidence.

## Confirmed user direction

Product name: Monitter. Custom harness identities are called agents. Mira is the approved SSH test host;
Codex is /home/alex/.npm-global/bin/codex (0.130.0), home /home/alex. Use a dedicated smoke-test folder.
Multiplayer is explicitly OUT of v1: no invites, accounts, shared remote service or collaboration backend.
A future version will let colleagues/friends join group channels and share selected agents. Keep channel
identity and membership separate from machine credentials; agent owners retain their harness credentials.
