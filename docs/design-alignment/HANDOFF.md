# Design alignment handoff

## Takeover, 2026-10-23

Implementation moved to the isolated worktree `/Users/alex/code/monitter-design-20261023` on branch `feature/design-alignment-20261023`. The shared `main` checkout remains untouched. Commits `cae8d68` and `80ec32f` apply theme-derived fill, hairline, text, glow and status tokens to shared controls, preferences, composer, work plan, diff rows, build queue, Ideas, Watch and agent activity. A later batch also aligns the dock strip and puts Work Plan beside Build Queue. These edits are awaiting a queued desktop compile check; they are not installed or visually verified. The older audit and task list below describe the state at handoff time, before these commits.

Current build request: `dcbde34d-04cb-4490-a0b0-0ca7819af025`, target `/Users/alex/code/monitter-gpui/.cargo-target/gpui`. The first request was cancelled before running because its command omitted the local Rust toolchain from `PATH`. The project allows one running build at a time; claim the current request before compiling. Do not infer a successful build from readiness.

Written 2026-10-23 15:50. Goal set at ~03:45: bring the GPUI desktop in line with the Claude design boards by 07:00. That deadline passed with the session idle; the goal is still open.

## Source of truth
- Design: `~/Downloads/Monitter Design System.html` (17 boards, bundled). The claude.ai artifact link was not shared with the agent, so the downloaded HTML was used.
- Unpacked copies for reading: `/tmp/ds/board-*.html` (volatile; re-extract from the HTML if missing: each board is a gzip+base64 entry in `__bundler/manifest`, template in `__bundler/template`).
- The boards say Geist fonts and the navy/cyan hexes are stand-ins. Token names, px values, percentages and recipes are what carry over.

## Constraints from Alex
1. All colour must flow through the theme system (presets, light/dark, configurable accent). No hardcoded board hexes.
2. The new topbarless chat panes stay topbarless. Ignore pane headers on chat panes in the boards.
3. Build in batches, not per fix. Queue builds through the build queue (target key `/Users/alex/code/monitter-gpui/.cargo-target/gpui`); ~45 GiB free at last look.

## Original handoff snapshot (superseded by takeover above)
- `desktop-gpui/src/design.rs`: added `Palette::fill(0..3)`, `tint()`, `hairline(0..1)`, `glow()`, `wash(base)`, `text(0..3)` and `mix_rgb()`. Alphas follow the board ladders (dark/light variants). Nothing calls them yet. Not built; run `cargo check` before relying on it.
- Audits written (this folder): `audit-core-boards.md`, `audit-pane-diff-queue.md`, `audit-watcher-ideas-activity.md`.
- Memory saved: `design-alignment-constraints.md`.

## Still open after the takeover commits
The design branch has not passed a desktop compile check or visual verification. The pane surface gradient, dock strip, full activity overview and several shared control details remain open. The source audit below is a work list, not a verified completion report.
`composer_footer.rs:1505` `0x3f9d6a` is the usage-ring green (status), leave it. `draft_creation.rs` has ~10 hardcoded colours and font "Montserrat"; unaudited.

## Suggested order
1. `design.rs`: add `ok()/warn()/err()/agent()` palette-derived status colours (used by pane_details, git_diff_view, build_queue, agent_exchange, chat_watcher, status_color). Do this first; other work depends on it. Small.
2. Batches by file ownership (no collisions):
   - A: shared controls (`settings.rs` apply_native_theme, vendor gpui-component input/button/checkbox/switch/slider/select): 36px inputs, 7% fill, line-2 edge, glow focus ring, fill(2)/fill(3) hover/active.
   - B: preferences window (`preferences_modal.rs`, `settings_general.rs`, `settings_appearance.rs`, card/toggle/nav parts of `settings.rs`).
   - C: chat pane parts: composer tiles/send (`chat.rs` 5690-5830, `composer_footer.rs`, `composer_git.rs`), turn row (`run_activity.rs`), status pill (`working_bubble.rs`), message/report cards (`chat.rs` 4560-4970; replace `FINAL_ANSWER_GREEN`).
   - D: pane shell wash + glow edge (`chat.rs:6604`, `docking.rs`, `surface_blend.rs`, `activity_sparkles.rs`), dock strip (`collaboration.rs`).
   - E: `pane_details.rs` work plan rows/shell, `git_diff_view.rs` line metrics + file list, `build_queue.rs`, `agent_exchange.rs`.
   - F: `chat_watcher.rs` (feed text sizes, theme-aware floating controls; update tests at ~2539), `ideas.rs`, `admin_activity.rs`.
   - G, last: `activity_overview.rs` cards and the 460px agent side panel (also touches chat_watcher and shell wiring).
3. One `cargo check` (desktop bin) after integrating each group of batches, then visual-export fixtures across several presets in light and dark. Existing test compile errors (browser.rs `mpsc`) are pre-existing and block `cargo test` for the desktop bin.
4. Defer: pane naming ("Codex #12") is a data-model change; backend fields for build duration/disk/live tail.

## Working-tree state
`main` has other tasks' uncommitted edits (chat.rs, chat_watcher.rs, collaboration.rs, main.rs, markdown_pane.rs, settings*.rs, web/iOS files). Use isolated worktrees for implementation batches and preserve others' changes. Roster report was filed for `design.rs` only.

## Process notes
Background subagents die when the session turn ends or restarts (two audit rounds were lost). Run agents in the foreground, or finish each turn only after they report.
