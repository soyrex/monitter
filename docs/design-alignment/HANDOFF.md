# Design alignment handoff

## Finishing audit, 2026-10-23

The older numbered audits contain 49 original observations, not 49 outstanding tasks. The branch now changes more than 30 project files and has completed the palette, main preferences/composer/work-plan/diff/queue/Ideas/Watch token pass, activity card and hub tones, dot grid, filters, cluster washes, legend, segmented controls, and search/zoom chrome. The segmented controls and search/zoom lines in the older “Still open” paragraph below are superseded by commits `a20a989` and `6078fa3`.

The remaining required work is organized into four reviewable batches:

1. Complete the activity agent panel and verify a visual frame. The embedded 460px panel now hosts the retained live Watch view, dims the surrounding map, has a theme-derived surface, agent/status/project/elapsed header, and closes by Escape or its button. Isolated desktop `cargo check` passed in run `bbec8972-8de0-47bc-919a-e1e53926198d`. The real Desktop fixture passed with the 32/32 focused Activity suite in run `cd350233-a288-40e0-83db-46cf2c010698`; a follow-up 32/32 pass in run `77286e3e-9e99-4866-9655-0c82aad9361f` verifies switching between two cards, repeated open/close, retained Watch updates and cleanup without changing the selected workspace. Final synthetic light/dark frames `artifacts/gpui/visual/gpui-frame-1792785087046901000.png` and `artifacts/gpui/visual/gpui-frame-1792785120449854000.png` were inspected: the panel follows each theme, the map dims, elapsed time is realistic, and the activity feed fits. They are not installed-app evidence.
2. Finish the pane shell effect: theme-derived glow edge/running field and a visual fixture containing an actual docked chat pane. The final synthetic docked chat frame `artifacts/gpui/visual/gpui-frame-1792785157420118000.png` visibly shows the top wash, active accent edge and sparse six-star field; it is not installed-app evidence. The branch now uses the 15px Tern pane radius introduced on canonical main, including rounded border geometry. The topbarless chat pane and user-selected tile style remain.
3. Resolve the remaining board details in shared form controls and the Watch/Ideas/activity surfaces. Keep application-specific controls and backend behavior where the boards only suggest a different feature. The compact Watch feed labels and times passed the focused `watcher_activity` suite (5/5, runs `bf2173d5-af25-4e7c-9558-821b844e9cfe` and `c70c83a7-6b31-4558-a1f1-83fece93954b`). Ideas status icons and avatars passed the focused Ideas suite (7/7, run `261c890c-8edd-4ab9-8064-de7cb32fa867`). The final visual pass confirms distinct Watch action labels and targets. Update the audit entries with done/deferred evidence as each batch closes.
4. Reconcile the branch against moving `main`, compile and review dark/light visuals across several presets, and validate the installed app only after integration. The branch has no release package or installation; existing synthetic frames are not live-app proof.

The goal is active. This list is a scoped finishing plan, not a claim that every old audit line still applies. The initial 03:45–07:00 deadline below expired and should not be used as a completion estimate.

## Takeover, 2026-10-23

Implementation moved to the isolated worktree `/Users/alex/code/monitter-design-20261023` on branch `feature/design-alignment-20261023`. The shared `main` checkout remains untouched. Commits `cae8d68` through `ce58834` apply theme-derived fill, hairline, text, glow and status tokens to shared controls, preferences, composer, work plan, diff rows, build queue, Ideas, Watch and agent activity. The dock strip now includes Work Plan beside Build Queue. The activity map keeps the Done success tone, uses the reference's finer dot grid, and has lighter state borders, hub fill and card shadows. The isolated desktop `cargo check` passed with exit 0 after copying generated web resources and matching OpenUI dependencies into ignored worktree directories. These changes are not installed. The older audit and task list below describe the state at handoff time, before these commits.

Completed compile-check run: `3991c502-a79d-4193-9e46-39bf3a2fe449`, target `/Users/alex/code/monitter-gpui/.cargo-target/gpui`. It ended with observed wrapper exit 0 and Cargo `Finished` in 23.63 seconds. Earlier checks exposed missing worktree web resources, missing OpenUI dependencies, and three source errors; all were resolved before this pass. The project allows one running build at a time; register and claim any further compilation.

The feature-enabled synthetic preview binary built with observed exit 0 (run `8a29ddc7-3784-4fd2-beed-e0bc8947e2e5`), then rebuilt after the theme and activity edits (runs `a5f38610-5ae0-4d3a-b26c-31db518f7d16` and `ee6dcb16-7287-4ad4-a90f-ff2a403f4b2d`, both observed exit 0). Dark and light overview, work plan and activity frames were exported and inspected. The controls follow the light/dark tokens; activity hubs show the full project name without the earlier muddy light-mode fill. No visible clipping was found in the captured viewports. Final activity frames: `artifacts/gpui/visual/gpui-frame-1792773735909953000.png` (dark), `artifacts/gpui/visual/gpui-frame-1792773753223844000.png` (light). These are synthetic frames, not proof of the installed app's appearance.

Focused `activity_overview` tests passed 32/32 with observed exit 0 (run `4754dd52-b96b-4bd4-b303-3b954e9f0c08`, 902 other tests filtered). Earlier attempts exposed five stale test fixtures and two assertions for fixed blue/grayscale colours; the branch updates those fixtures and assertions. The activity test proves render and interaction behavior in the fixture, not live app behavior. The branch has no release package or installation.

The next pane shell batch gives originally opaque chat and retained non-browser panes a single theme-derived top wash that fades to their existing fill at 65% height. The browser pane and translucent/glass panes retain their previous backgrounds and blend behavior; focus and inactive emphasis still use one background layer. The desktop check passed with observed exit 0 (`33694a3a-4d54-43e0-9105-a70e2c7a5ba5`, Cargo 43.44s), and the feature-enabled preview build passed (`8c3a355d-65f7-4c66-89d0-68f96a384b5c`, Cargo 1m55s). Existing synthetic screens do not host a docked chat pane, so the wash itself has not been visually verified yet.

The activity header now uses 28px rounded filter chips, theme hairlines and fills, semantic state dots, mono counts and a divider beside the title. Its feature-enabled preview build passed with observed exit 0 (`af0a1049-7195-4ba1-a03f-2db753aa3d64`, Cargo 1m39s). Captured light and dark synthetic activity frames are `artifacts/gpui/visual/gpui-frame-1792775832969228000.png` and `artifacts/gpui/visual/gpui-frame-1792775846132049000.png`; both were inspected and the header remained readable without clipping. Neither frame proves installed-app behavior. No focused activity test was rerun after this presentation-only header edit.

The activity map now paints a theme-derived project wash beneath links and cards with four low-opacity circles per group. Its preview build passed with observed exit 0 (`72a911c9-3361-44b2-8375-1a27ff3d8772`, Cargo 1m35s). Light and dark synthetic frames `artifacts/gpui/visual/gpui-frame-1792776223644103000.png` and `artifacts/gpui/visual/gpui-frame-1792776246346255000.png` were inspected: project tint remains subtle and labels, links and cards remain legible. The circles are an approximation of the board's radial gradient; slight rings are visible in the light frame.

The activity footer now uses the board's 40px minimum height, 20px spacing and inset, 6px semantic dots and theme hairline. Its legend describes the dashed working links and dotted communication links without assuming a blue theme. Project links now draw a 1px dashed accent when working and a faint theme hairline in other states. The right-side keyboard hint remains open because a card-keyboard action has not been verified. The feature-enabled preview build passed with observed exit 0 (`a1cd3c50-839b-440a-aa7c-c75baf129a6a`, Cargo 1m39s). Light and dark synthetic frames `artifacts/gpui/visual/gpui-frame-1792776677105897000.png` and `artifacts/gpui/visual/gpui-frame-1792776720103796000.png` were inspected: the legend fits on one line at 1160px and the subtle project links remain visible. This does not verify the installed app.

The activity grouping and Clusters/List view controls now share compact outlined segmented containers with theme-derived selected fills. The feature-enabled preview build passed with observed exit 0 (`8eeadd87-fde1-4abe-8994-9d0d103a983f`, Cargo 3m13s). Light and dark synthetic frames `artifacts/gpui/visual/gpui-frame-1792777137918972000.png` and `artifacts/gpui/visual/gpui-frame-1792777185153445000.png` were inspected: both groups fit in the 1160px preview header with readable selected states. Search and zoom controls still need the board's pill and icon treatment. This is not installed-app verification.

The next small control batch places search in a theme-lined 200px pill with a search icon and gives the three zoom actions 28px outlined controls. Its feature-enabled build passed with observed exit 0 (`5976e89b-5808-4b36-82c9-a4b3785deb03`, Cargo 1m36s). Light and dark synthetic frames `artifacts/gpui/visual/gpui-frame-1792777481595116000.png` and `artifacts/gpui/visual/gpui-frame-1792777524996311000.png` were inspected: the search placeholder and controls remain legible, and the header fits at 1160px. This does not verify keyboard focus, interaction or the installed app.

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
The design branch has passed desktop checks, focused activity tests and synthetic visual review. The pane wash needs a fixture or installed-app visual check; its glow edge and running field, the complete activity side panel and several shared control details remain open. The source audit below is a work list, not a verified completion report. See the newer finishing audit above for current status.
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
