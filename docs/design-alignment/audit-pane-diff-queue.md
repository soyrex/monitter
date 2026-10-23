# Audit 2: workplan, diffs, buildqueue, exchange (read-only, no builds)

Paths are under /Users/alex/code/monitter/desktop-gpui/src. Sizes: S = under 1h, M = a few hours, L = a day or more.

## Cross-cutting (do first, unblocks all four)
- Palette (design.rs:380-440) has no ok / err / warn / agent / diff-wash tokens. Boards use them everywhere. Code hardcodes hex in four places:
  - pane_details.rs:149-157 (0x3eaa7a, 0xc96058)
  - git_diff_view.rs:19-31 (0x34d399 / 0x087f5b, 0xff5c6c / 0xc73545)
  - build_queue.rs:98-99 (the same hex pairs)
  - agent_exchange.rs:79,89,253,428,445,483 (0x6594db, 0xd35e5e, 0x34d399 / 0x147d59)
- Fix sketch: add `Palette::ok()`, `err()`, `warn()`, `agent()` to design.rs.
  - ok / err / warn: fixed status hues, with light and dark variants, mixed 10-15% over paper through `mix_rgb`.
  - agent: a fixed blue (board `#5b86ff`) tinted a little toward the accent.
  - Optional `Palette::pill(color)`, returning color at 12% over paper.
- Then replace all the hex above. Size: S-M. Files: design.rs (this one first), then each board's files.
- The board 14px radius, glow shadow `0 0 32px signal@8%` and the 1px top sheen (ink 4%) on glow popups: check whether a shared helper exists (design.rs `space::MODAL` = 14). Add `glow_card()` if not.

---
## 1. board-workplan (pane_details.rs, render_work_plans 771-1060)
Where it lives: today it is a card inside the Details pane (pane_details.rs:1309). I found no floating "Working pill" popup in GPUI. The board specifies a 400px popup, 8px above its trigger, closing on outside click. Treat the popup shell as a separate task (Large) and fix the content below first, since it works in both places.

Ranked gaps:
1. **Plan rows (M)**
   - Board: 14px step text, 16px icon, row `py 12`, hairline between rows (line-1), gap 12, state label 12px at the right. Pending rows use tx-2 text and a tx-3 empty ring. In-progress is a ring with a dot in signal. Done is a check in ok, stroke 1.8.
   - Code (pane_details.rs:884-905): 11px title, 13px icon, `py 6`, gap 7, 9px label. Icons are generic IconName (Check, CircleDot, Circle). Pending text is full ink.
   - Fix: change the sizes. Pending title colour `palette.text(1)`, label `palette.text(2)`. Done label uses ok. The in-progress icon needs a custom ring-and-dot (`svg()` or two nested divs, 16px). This is the single most visible gap.
2. **Card shell (S)**
   - Board: radius 14, 1px border `glow()` while active or `hairline(1)` when complete or closed. Background is a gradient from `wash(panel)` to panel (180deg, 65% stop). Outer glow 32px at signal 8%. Header padding `20 20 4`, gap 12.
   - Code (:818-826): radius 7, `p_3` (12px), border `palette.line`, flat `surface_color(panel)`, `gap_2`.
   - Fix: radius `space::MODAL` (14), padding 20, `linear_gradient(180, wash→panel)`, and `.shadow` with accent at 8%.
3. **Header block (S)**
   - Board: "WORK PLAN" in mono 11 with .12em tracking, tx-3. Status "Active" 13px in signal, or "Complete" in ok. Title 17/500 with -.01em. Count line 12px tx-3.
   - Code (:828-866): 10px, 10px, 13px, 10px. Status says "Active" or "Closed" and never "Complete". Count line text is OK.
   - Fix: bump the sizes, add a Complete state when completed == items.len(), and use tracking.
4. **Segmented bar (S)**
   - Board: 5px high, radius 3, gap 4, fill ok / signal / `hairline(1)`-like (rgba .22 ≈ `fill(3)`/tint). Only one segment is cyan.
   - Code (:911-942): 5px high, but gap 2 and `rounded_full`. The bar has a track background, and pending is `mix(panel, muted, .19)`. The colours are hardcoded.
   - Fix: gap 4, radius 3, drop the track. Done → `palette.ok()`, current → accent, pending → `hairline(1)`. Remove the container `rounded_full`/`overflow_hidden`.
5. **Earlier-plans footer (S)**
   - Board: full-width 40px row at the bottom of the card, top hairline, 13px tx-2 text, centred, 16px chevron. Hover is `fill(2)` with tx-1, radius `0 0 12 12`. Expanded shows a down chevron.
   - Code (:1000-1010): compact text button, 10px, `pt_2`, left-justified, 12px chevron. Chevron logic is correct.
   - Fix: `h(40)`, `w_full`, centre, 13px.
6. **Earlier plan rows (M)**
   - Board: flat row `py 12 / px 20`, top hairline. A state icon (green check if all done, or a dashed grey ring if closed with steps left) sits beside a 14px/500 truncated title. Under the title: a 36px mini bar (3px segments, gap 2) plus "N of M done · Closed" at 12px tx-3. Right side: ghost "Outcome" and "Info" buttons (12px tx-3, `padding 5/8`, radius 6). Row hover is a 5% wash. More than 5 rows scroll inside the popup.
   - Code (:1012-1050): a title plus a status string (raw `status` text) and a 160-char summary. No icon, no mini bar, no actions, no hover.
   - Fix: new `earlier_plan_row()` helper. The dashed ring needs `border_dashed` or a custom path. Outcome and Info need to be wired to the existing summary/info flows.
7. **Show-more items button and summary box (S)**
   - Code uses 9px text. Move to 12px tx-3. The summary box uses `palette.soft` at 10px, so use `fill(1)` and 12px.
8. **Not on the board, keep:** blocked and skipped states (:798-805). Give them err and a tx-4 dash, and keep them out of the "only one cyan" rule.

Files touched: pane_details.rs (all of this), design.rs (ok / err tokens). Total about M-L.

---
## 2. board-diffs (git.rs, git_diff_view.rs, git_diff_model.rs)
The code uses `ratio = font_size / 14`, so board px values below are at ratio 1. The code's base sizes are 10px mono with a 14px line, and the board is 12px with a 20px line.

Ranked gaps:
1. **Diff line metrics (S, git_diff_view.rs:161, 183-210, 253-310)**
   - Board: 20px line, 12px mono. Gutters 44px each, right-aligned, `pr 10`, tx-3. Sign column 20px. Code `pr 16`. Context text tx-2.
   - Code: `text_size 10*ratio`, `line_height 14*ratio`, `min_h 16*ratio`, gutter `30*ratio` with `pr 4`, sign `14*ratio`. Context is `c.muted`, which is darker than tx-2.
   - Fix: 12 / 20 / 44 / 10 / 20. The context colour goes to `p.text(1)` and the gutter number colour to `p.text(2)`. In the split view, drop the number columns from 44 to about 36 so two sides fit.
2. **Hunk header (S, git.rs:1298-1313)**
   - Board: 24px high, `padding-left 64` (aligned with the code column), mono 11, signal text, bg signal 7%, name of the enclosing function after the `@@`.
   - Code: `min_h 20*ratio`, `pl 8*ratio`, wrapped. The 7% bg and signal colour are already correct. Header text includes the function name only if the model keeps it (check git_diff_model.rs:197).
   - Fix: `h(24)`, `pl(64)`, `whitespace_nowrap`.
3. **Fold rows (S, git.rs:1316-1331 and 1270-1296)**
   - Board: 24px bordered bar (top and bottom hairline, line-1), mono 11 tx-3, centred, a 12px chevron and "38 unchanged lines". Hover is `fill(2)` + tx-1. Folds appear after 6 unchanged lines (model already folds at len-6, which fits).
   - Code: text-style Buttons with a fixed `available` width and no chevron or borders. The "expand context" gap button uses a different label.
   - Fix: share one `fold_row(label)` helper with `h(24)`, borders, chevron and hover. Use the same helper for the "Show remaining N hunks · M changed lines" footer (:1379-1390; code says "Show remaining {} hunks" with no line count) and "Show more".
4. **Colours (S, git_diff_view.rs:19-31, 172-210)**
   - Add/del wash 10% and gutter wash 16% match the board. Word highlight is 30% (matches).
   - Gap: colours are hardcoded hex and the light/dark switch is computed from paper sums. The sign column on a changed line should use ok / err and context should use tx-3, which the code does (`tint.unwrap_or(muted)`).
   - Fix: `DiffColors::new` should read `p.ok()` and `p.err()` from design.rs.
9. **File list rows (M, git.rs:760-870)**
   - Board: 36px row, padding `0 20`, gap 10, 13px sans. Status chip 18px, radius 5, line-2 border (A uses ok at 45%, D uses err at 45%). The directory part is tx-3, and the name is 500 weight. **Deleted names have a strike-through.** Counts are mono 11. A 5-block bar uses 6px blocks with 1px radius and 1px gap. The selected row is `fill(3)` ("--active"), not an accent tint. Lockfile and generated rows show "Generated · 4,210 lines hidden".
   - Code: 26px row, 10px text, `px 12`, gap 8. Blocks are 4px with opacity 0.8. Selected is accent at 14%. The 18px chip and radius 5 match. No strike-through on D. Generated rows show only "Generated" or "Lock file". The chip colour for M is `c.muted` at 40%, and board uses line-2 and tx-2.
   - Fix: sizes, `fill(3)` selection, strike-through for D via `.line_through()`, line count in the generated label (needs the count from the stat), 6px blocks.
10. **Changes drawer header (M, git.rs:690-720 and where the drawer is built)**
   - Board: header `16 20 14`, "Changes" 13/600, a branch glyph + branch name (mono 11 tx-3), right-aligned "42 files · +1,204 −388" (mono 11), and a 4px two-colour churn bar (ok/err, flex proportional to totals) under it. Footer fold "34 more files". Drawer card: radius 14, glow border.
   - Code has `churn_bar` (3px, `w_full`) but it is used per file where the board has it only in the header. Check whether a totals header exists; I did not find one. Also, files are sorted by churn already (test at :1463), which fits.
   - Fix: add the totals header and bar. Size M.
11. **Diff header (S, git.rs:1000-1130)**
   - Board: 44px high, `0 12 0 16`, chevron + chip + path (13px, name 600) + counts. A Unified / Split segmented control in a 1px line-2 frame, `padding 2`, 22px segments, radius 6, selected is `fill(3)`. Icon buttons 28px, radius 8.
   - Code: panel surface header with `py 8`, `Button::text` toggles. "WS N" toggle is not in the board (keep, but demote). No segmented frame.
   - Fix: build a `segmented(["Unified","Split"])` helper. If other boards have one (`board-components`), share it.
12. **Inline edit in a turn (S, git.rs:1515-1536)**
   - Board: radius 12, `raised` bg (`fill(1)`), line-1 border, 32px header with chip, path and counts, then at most 1 hunk / 6 lines, then a 28px "Open in Changes" fold-style footer.
   - Code: radius 6, no header, no footer, no raised bg.
   - Fix: add header and footer rows around the existing code rows. The footer needs a callback to open the Changes drawer for that path.

Files touched: git_diff_view.rs (1, 4), git.rs (2, 3, 9-12), design.rs (colours). Split: git_diff_view.rs + part of git.rs render_diff (lines 962-1400) can go to one worker, git.rs file list (lines 600-960) to another, to avoid conflicts in the same file.

---
## 3. board-buildqueue (build_queue.rs; the pane is built in collaboration.rs:1242-1420)
build_queue.rs already has the three-part structure (title + pill + time, task meta, details disclosure). Remaining gaps:

1. **Key-fact chips and report summary (M, build_queue.rs:263-336)**
   - Board: after the meta line, a chip row (`gap 6`): "exit 0" (with a 6px ok dot), "1256.6 s", "build 1792065927651", "90.8 GiB free". Chips are 20px high, `px 7`, radius 6, line-2 border, mono 11, tx-2. Then a 2-line-clamped report (12px / 1.55, tx-2). Then the Details toggle.
   - Code: only an "Exit N" code block (`code()`: radius 4, `palette.code` bg, 10px mono) and an 11px muted clamped summary. No duration, build id or disk chips. These need to be parsed from fields on `run` (durations from startedAt/finishedAt; disk and build id may only be in the summary prose, so ask the backend, or regex).
   - Fix: add a `chip(text, dot: Option<color>)` helper and render the chips.
2. **Details well (S, :263-336, 245-258)**
   - Board: label/value grid `92px | 1fr`, gap `6 12`, padding `12 14`, radius 10, `fill(1)` ("raised") bg and line-1 border, mono 11 with 1.5 line height. Labels tx-3, values tx-1. The Note value is sans 12 tx-2.
   - Code: 2-column grid with stacked label-over-value cells, `p 10`, radius 8, `surface_color(panel)` and no border. Labels 10px, values 11px.
   - Fix: switch to a true dt/dd grid (`grid_cols` with a fixed first column; GPUI `grid_cols(2)` is equal-width, so use a flex row per pair with a `w(92)` label). The "Build report" row spans the full width with sans 12. Rename labels to Exit / Duration / Check / Requests / Fingerprint / Disk / Note to match.
3. **Section headers (S, :60-66)**
   - Board: a 36px bar, `padding 0 20`, mono 11 with .08em tracking, tx-3, a bold name in tx-2 (weight 500) and a count ("BUILDING 1", "RECENT BUILDS 3"), bottom hairline.
   - Code: 10px semibold muted text, `pt 7`, no count, no bar, no divider.
   - Fix: use a shared `section_bar(title, count)` helper.
4. **Row padding and title line (S, :395-442)**
   - Board: `.brow` `padding 14 20`, gap 8. Title 14/500 and tracking -.005em. Title, pill and time on one line (gap 12), with the pill and time `flex: none`. Row hover is a faint wash (hover at 45%). Cancelled title uses tx-2.
   - Code: `py 10`, no horizontal padding (the parent supplies it?), gap 5, flex_wrap title that wraps pill and time below at 140px min. Add `hover` and the cancelled colour.
5. **Status pill (S, :96-140)**
   - Board: 22px, `px 8`, gap 6, 11/500. Building has a 1px border of signal at 35% over a signal-soft (14%) fill. Succeeded is ok at 12%. Cancelled is tx-2 over `fill(1)` with a clock. Failed is err at 12% with an X. **Queued is warn at 12% with dots.**
   - Code: `px 6`, gap 4, fill 13% for all states. No Building border. "ready" and "planned" map to the primary colour with a check ("Ready to build" and "Preparing"). Succeeded and Failed hex are hardcoded.
   - Fix: add the border for running, use `warn()` and a dots icon for planned and ready (label "Queued"), and move colours to the palette. The time label drops the date for today (already done in `timestamp_label`). Time should be mono 11 tx-3, but 2m 14s elapsed (not a clock time) for the active build, in tx-2. Today the badge shows the start time, so add elapsed time for running rows.
6. **Live bar and latest output on the active row (M)**
   - Board: a 3px track (line-2) with a 38%-wide signal segment sliding (`mt-slide` 1.6s ease-in-out infinite, translate -100% → 260%). Below it a "Latest output" toggle and a mono 11 log tail (the last 3 lines; tx-3 for old lines, warn for warnings).
   - Code: none. The spinner in the pill is the only motion. Needs `with_animation` and a backend field for the output tail (probably missing, so ask first or hide).
7. **"N requests combined" chip (S, :338-358)**
   - Board: a chip on the right of the meta line. 20px, radius 6, line-2 border, mono 11, text "2 requests combined".
   - Code: `px 6 / py 2`, radius 8, panel bg, "2 combined". Use the shared chip.
8. **Section naming (S)**: code has "Waiting requests" and "Recent requests" extra sections. Board shows only BUILDING and RECENT BUILDS. Keep them but style them the same.
9. **Pane tab strip in the board (dock buttons "Subagents / Agent map / Work Plan / Build Queue (4)")**: out of scope for this file (collaboration.rs). The board's active dock button uses signal-soft with a 50% signal border. Check collaboration.rs:1314 (`pending_count`).

Files touched: build_queue.rs (1-8). collaboration.rs only for item 9. design.rs for tokens. Total about M.

---
## 4. board-exchange (agent_exchange.rs, plus the call site in chat.rs:5236-5380)
The collapsed marker is structurally right (28px, no fill, direction glyph, dashed line, mono time). Gaps:

1. **Agent colour is not from the theme (S, agent_exchange.rs:79, 428, 445, 483)**
   - Board: `--agent` blue, with `agent@28%` for the dashed line and rail, `agent@12%` for the expanded edge, `agent@5%` for the wash, `agent@7%` for the hover.
   - Code: hardcoded `0x6594db` and the failed colour `0xd35e5e`. Move to `palette.agent()` and `palette.err()`.
2. **Dashed line colour (S, :221-227)**: code uses `palette.line` (10%); board uses agent at 28%. Change to `agent().opacity(0.28)`. Failed state should be a broken line in err (board); today it is dashed in `ink`, which is the err colour already (only the glyph).
3. **Info icon and chevron (S, :222)**: board says "The info button is gone; the whole row is the toggle" and shows a 12px chevron at the right in tx-4 (`.ch`). Code shows `IconName::Info`. Replace with ChevronRight / ChevronDown.
4. **Collapsed row text (S, :179-215)**
   - Board: name in tx-2 / 500 ("Codex #12"), then event in tx-3 ("delivery started"), 12px (`.xm`). Time is mono 11 in tx-4 as HH:MM with no seconds, plus no date. Gap 8. Hover is a rounded-6 agent 7% wash.
   - Code: 11px, gap 6, `palette.muted` for all text, time `%H:%M:%S` (seconds belong only in the expanded view). The ghost Button supplies its own hover, not agent-tinted.
   - Fix: `%-I:%M` or `%H:%M` (keep the locale rule used elsewhere in the app), split name and event styles, 12px.
5. **Expanded container (M, :229-312)**
   - Board: `.xo` has a 5% agent bg, a 1px 12% agent border, radius 8, margin `6 0 10`. The header row is 32px, `padding 0 8`. The flow area has padding `8 12 12 18` and a 1px rail at 28% agent from the first dot to the last. Steps are 9px dots with 12 gap, `py 5`, 12px tx-2, time mono 11 tx-4 at the right. States: hollow (window colour fill + agent border) for the first (sent), filled agent for the middle, ok for the final "completed" step. Then a payload box (`.pl`: margin-left 21, radius 6, white 4% bg, mono 11, tx-2, "Asked: ..."), and an action button ("Open Tray fix": 12px, `padding 4/6`, radius 6, hover `fill(2)`).
   - Code: no wash and no border, padding `12 / 10 / 12`, gap 8, steps with a 7px dot (first hollow, others filled: the same logic) and a rail at 24%. Time is `%H:%M:%S` in mono 11 (matches the board, since the board shows seconds when expanded). Step labels use muted, not tx-2. No payload box (details are the member rows via `render_item`) and no action button.
   - Fix: wrap `content` in the wash + border + radius 8 box, dot 9px, rail 28%, label tx-2. Payload box and "Open <peer>" action are new (Medium; need the collaboration request text, which is probably in the record, and a pane-focus callback).
6. **Name chip with sequence number (M, project() at :30 and chat.rs agent names)**
   - Board: "Codex #12" default, custom name leading with the default id in mono tx-4 beside it. The same component in tabs, header, marker, Agent map and Subagents. Truncate in the middle past about 24 characters. Rename by double-clicking the title.
   - Code: uses the agent `name`, no sequence numbers. This is a data-model change (per-harness counter, never reused), so it is Large and touches several files. Suggest deferring it, and then shipping only the marker style.
7. **Run collapse text (S, chat.rs:5340-5360)**: the group label reads "Codex · 3 exchanges". Matches the board. "Agent activity · N tools · M exchanges" is an addition, fine.
8. **Three tiers (S, outside this file)**: the board's turn log is 22px, 11px tx-4, no glyph (`Turn · 5 tools: Ran a command` + time at the right). Check tool_detail.rs and the turn-log row in chat.rs against it; I did not audit them.
9. **Messages**: the board's message card has a teal-ish wash `rgba(60,200,190,.06)`, radius 12, padding `12 14`, avatar gradient 20px radius 6. Out of scope here (topbarless panes unaffected). Don't hardcode the teal; if wanted, use `wash`.

Files touched: agent_exchange.rs (1-5), chat.rs 5236-5380 only if item 7 or 6 are done, design.rs for `agent()`. About M, plus L if the numbering is included.

---
## Suggested split (no file collisions)
- Worker A: design.rs tokens (ok / err / warn / agent, `pill()`, `glow_card()`), first.
- Worker B: pane_details.rs (workplan items 1-7).
- Worker C: git_diff_view.rs + git.rs lines 962-1400 + 1515-1536 (diff view, hunk, fold, inline edit).
- Worker D: git.rs lines 600-960 (file list + Changes drawer header). Assign to C if one person avoids merge pain.
- Worker E: build_queue.rs (items 1-8).
- Worker F: agent_exchange.rs (items 1-5), and chat.rs only if the sequence-number naming is approved.

## Top 8 by visible impact
1. Work plan row sizes and ring icons, and the card gradient / glow (pane_details.rs) (M)
2. Diff line metrics 12/20/44 (git_diff_view.rs) (S)
3. Build queue chips + details well + section bars (build_queue.rs) (M)
4. Exchange expanded wash, 9px dots and removing the Info icon (agent_exchange.rs) (M)
5. Fold and hunk rows (git.rs) (S)
6. File list row 36px, selected state, strike-through for deletes (git.rs) (M)
7. Status pill border / Queued state (build_queue.rs) (S)
8. Palette ok / err / warn / agent tokens, then remove all hardcoded hex (S-M)
