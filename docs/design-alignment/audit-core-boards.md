# Audit 1: components, agentpane, forms, prefs boards vs desktop-gpui (read-only)

Paths are relative to /Users/alex/code/monitter/desktop-gpui. Palette helpers fill()/hairline()/tint()/glow()/wash()/text() in src/design.rs:401-440 are currently UNUSED outside design.rs (uncommitted). Every surface still uses legacy slots (palette.card/panel/soft/line/muted/accent). That is the root cause of most gaps below.
Board ladder: fill 3.5/7/10/15, hairline 10/18, text 92/66/46/30, glow 38, wash 14 (src/design.rs already matches).
Chat panes stay topbarless: board pane-header rows (44px title + more/max/sidebar/close) are intentionally skipped.

Sizes: S = under about 40 lines, one file. M = several call sites or one file with logic. L = cross-file or vendor.

## Ranking (visible impact first)

| # | Gap | Size | Files |
|---|-----|------|-------|
| 1 | Shared form controls (input, select, button, checkbox, switch, slider) do not use the fill/hairline ladder or the focus recipe | L | src/settings.rs (apply_native_theme 615-700), vendor/gpui-component/src/{input/input.rs, checkbox.rs, button/button.rs, switch.rs, slider.rs, select.rs} |
| 2 | Preferences window chrome (modal edge/header/Done, nav, cards, segmented, slider readout) | M | src/preferences_modal.rs, src/settings.rs (card/toggle/nav), src/settings_general.rs, src/settings_appearance.rs |
| 3 | Composer (tiles, send, focus glow, context bar, branch row) | M | src/chat.rs 5690-5830, src/composer_git.rs, src/composer_footer.rs |
| 4 | Pane surface: flat paper vs 14% tint wash gradient, radius 8 vs 14, glow edge | M | src/chat.rs 6604, src/docking.rs 365-420, src/surface_blend.rs |
| 5 | Turn row (10px, opacity, no hover fill, failed count uncoloured) | S | src/run_activity.rs 1508-1600 |
| 6 | Status pill (border, size, stop glyph, no failed state) | S | src/working_bubble.rs 195-345 |
| 7 | Dock strip (34px/radius 4/10px text) and missing Agent map / Work Plan entries | M | src/collaboration.rs 1248-1330 |
| 8 | Message header + report card (hardcoded green, timestamp format) | S | src/chat.rs 4640-4960 |
| 9 | Footer row (access / JEV / model) sizes | S | src/composer_footer.rs 910-1060 |
| 10 | Tooltip, modal/confirm/scrim, workspace edit popover | M | vendor tooltip.rs, src/preferences_modal.rs, src/workspace_tabs.rs 1385-1470 |
| 11 | Sparkle field counts/sizes/timing | S | src/activity_sparkles.rs |

Collision groups for splitting work:
- A (theme + vendor controls): #1, #10-tooltip. Touches settings.rs apply_native_theme and vendor/. Do first or in isolation; others may rebase onto it.
- B (preferences): #2. Touches preferences_modal.rs, settings_general.rs, settings_appearance.rs, and settings.rs card/nav fns only (distinct line range from group A, but the same file; coordinate).
- C (chat pane): #3, #5, #6, #8, #9 are separate files except chat.rs (#3 and #8 both in chat.rs; different line ranges).
- D (pane shell): #4, #7. docking.rs, collaboration.rs, surface_blend.rs.

## 1. Shared controls (forms board)  [L]  ,  highest impact
Board: field label 12/500 tx-2 above, gap 6. Input 36px high, r8, pad 0 12, fill-2 (7%), 1px line-2, 13px; hover fill-3; focus = border glow(38%) + 0 0 0 3px accent@14%. Mono variant 12px for hex/paths. Disabled opacity .45; read-only transparent + dashed edge.
Code now:
- apply_native_theme sets colors.input = surface_color(palette.card) (settings.rs:632). The vendor Input uses theme.input for BOTH background and border (vendor input/input.rs:774-776, theme/mod.rs:467), so fields have no visible edge and the fill is the card colour, not an ink wash.
- Medium input height is h_8 = 32 (vendor sizing.rs:272) vs 36.
- theme.colors.ring is never set (grep: no assignment), so focus ring = framework default; vendor focus_ring opacity 0.5 (styled.rs:13) vs 14% ring + 38% edge.
- Hover/active for buttons are left at library defaults (button_hover/active/secondary only get alpha rewritten, settings.rs:646-690), not fill(2)/(3).
Fix sketch:
- In apply_native_theme set: input -> surface_color(fill(1)); add/border via hairline(1) (needs a separate `input_border` token or change vendor input.rs:776 / checkbox.rs:238 to use theme.border-strong); ring -> glow(); button_hover/secondary_hover -> fill(2); *_active -> fill(3); selected/active -> fill(3) with hairline(1) edge.
- Vendor focus_ring: 3px, ring colour accent @ 14% (not 0.5 of ring).
- Make input size Large(36px) default for Input/Select in forms, or add a Size::Form=36.
- Button: primary = accent border @60%, accent wash @10% (hover 18%), text near-white; this is currently solid `.primary()` in several places (settings_general.rs:~96 "Save name"; design_system_preview.rs:Save uses `.primary().outline()` which is closer). Heights 28/32/40, r8, 13/500 (sm 12). Destructive: err border @45%, wash 8%, text lighter err; ghost: transparent, tx-2, hover fill-3.
- Checkbox 18px r5 / radio 18px round: unchecked fill-2 + line-2, checked accent@60% border + accent@14% fill + accent check (vendor checkbox.rs:238-312 uses theme.input for border and primary solid for checked; radio.rs same).
- Switch 36x20 r10, thumb 14: off = fill-2 + line-2 + tx-3 thumb; on = accent@60% border, accent@18% fill, accent thumb (vendor switch.rs:175-182 use tokens.switch/switch_thumb defaults; size not verified).
- Slider: track 4px line-2, fill accent, thumb 16px tx-1 with line-2 border (vendor slider.rs:165-170,217: bar_color.opacity(.5/.2), thumb ring animation, extra hover ring not on board).
- Select/palette swatch: 36px, 26x16 r3 swatch pair (paper + accent) before label, chevron tx-3 right; open menu radius 10, pad 4, rows 32px r6, hovered row fill-3, selected shows check. Current settings_appearance.rs:314/329 uses Select::new with PaletteChoice colors [paper, panel, accent] (3 chips; board shows 2: base + accent).
- Searchable select: match highlight 20% accent wash (not bold), group labels mono 10 .08em, footer "3 of 14 / key hints", 5.5 rows max; "No match" row with Clear search ghost button; Searching spinner row. Not found in code as a component: palette_search.rs / palette_catalog.rs exist, check whether Select with search is used; if not, M additional.
- Helper/error text: 12px tx-3, error replaces hint with icon in err-light colour; 1px err@60% edge on the input. Not present as a pattern; settings code uses ad-hoc 0xbd655b (settings.rs:1405, 1423) which should come from a palette error slot.

## 2. Preferences window (prefs board)  [M]
- Modal edge: board = glow(38%) border + 0 24 64 black@50% + 0 0 32 glow@8% + 1px inset sheen ink@4% top; surface = pane gradient. Code: border = palette.line (preferences_modal.rs:84), shadow 18/48 .4 + 4/12 .2 (:87-90), flat modal_surface_color(paper). Fix: border_color(glow), add accent glow shadow, sheen = 1px top border ink@4% (impl board says use top border not inset shadow).
- Header: board 56px, pad 0 20, title 15/600, Done = primary small (accent outline 60%/10% wash). Code: py INSET(10) px SECTION(20) ~ 40px, Done = plain compact Button (:92-95). Fix: h 56, `.primary().outline()` small style from #1.
- Search row: board 12/20 padding row beneath header, 36px input with 16px leading icon at 34px pad. Code: 12px pad block at top of content (settings.rs:1363) without leading icon. S.
- Nav: board width 248, pad 4/12/12, item 32px r8 13px gap 10, active = fill-3 + line-2 edge + ink text + accent icon, hover fill-2; group labels mono 10px .08em caps tx-3, pad 16/10/6. Code: width 224 (settings.rs:1333), active = accent@13% bg + accent text (:1319 `0x21`), ghost button py5; group label = proportional 0.71*size semibold muted (:1303). Fix as listed. S-M.
- Page: board title 20/600 -.01em, subtitle 13 tx-2, pad 28/36, cards gap 16, max width 1296 window. Code title = 1.5*size semibold (settings.rs:1371), hint 0.8 size muted; pad 28 ok; content max 760 centered (board fills width; keep or relax). S.
- Card: board r12, 1px line-1, fill-1 (3.5%), pad 20, gap 14, h3 14/600, p 12/1.45 tx-3; single-control card puts the control right of its title on one row (Appearance mode). Code: settings_card (settings.rs:39-47) pad 18, border palette.line, bg surface_color(palette.panel) (solid panel, not ink wash), title stacked, controls below. Fix: bg fill(0) wash, pad 20, and add `settings_card_row` variant for single control.
- Rows in a card: py 12, 1px hairline between, label 13/500 + 12 tx-3 caption, switch right (board). Code settings_toggle py 5, no dividers (settings.rs:48-54). S.
- Segmented control: board inline-flex (fit content), pad 3, gap 2, r10, 1px line-1, fill-1; segment 28px r7 13px; selected = fill-4 + line-2 edge + 14px accent check. Code settings_general.rs:44-72: full-width (`w_full`, `flex_1` buttons), gap 3, r7 container, bg palette.soft, selected = ghost `.selected()` default look, no check glyph. Fix: drop w_full/flex_1, r10, gap 2, selected style + check icon. S.
- Slider rows: board label left + mono 12 tx-2 readout right ("25%"), then track, then "Opaque ... Reset to default (ghost sm)" footer. Code prints "Window transparency: 25%" as plain text above (settings_appearance.rs:215-225) and a footer with a compact Button "Reset to default · 18" (:117-135). Fix: mono readout, ghost-sm reset, remove the default number. S.
- Light/Dark palette selects as 2-column grid inside "App theme" card: matches structure (settings_appearance.rs:314-329); only swatch/menu style differs (see #1).
- Backdrop/scrim: board base@60%; code black 0x99 (60%) + backdrop blur 20 (preferences_modal.rs:73). Use palette.paper at 60% instead of black. S. Entry 120ms fade: already (preferences_modal.rs:50).
- Small edit modal (Workspace) and destructive confirm: workspace editor is a Popover with header("Workspace", subtitle) (workspace_tabs.rs:1451) with r14; board modal footer = Remove workspace… (danger ghost, left) | Cancel | Save (primary), divider lines, pad 14/20, subtitle 12 tx-3. Verify the popover body footer in src/workspace_tabs.rs near edit_named_workspace; destructive confirm modal (alertdialog, Cancel focused first) is marked PROPOSED on the board; check if removal confirms exist (not found by grep "Remove workspace"). M.

## 3. Composer (components + agentpane boards)  [M]  , src/chat.rs 5690-5830
- Attach / dictate: board 40px filled tiles (fill-2, line-1, r8, icon tx-2 16px). Code: 32px (design.rs:132 COMPOSER_CONTROL_SIZE) ghost compact Buttons (chat.rs:5713-5718, voice_controls). Impl board: composer tiles 40 desktop. Fix: 40px tile with fill(1) + hairline(0); row height 56 with 8px side pad (code: p INSET=10, items_end, min_h 32). M because auto-growing Textarea must keep min 40.
- Send: board 40px, r8, 1px accent@60%, fill accent@8%, icon accent, glyph corner-down-left 16px. Code: 32px outline Button with `border_color(palette.accent)` and text accent at 100% (chat.rs:5783-5790), no wash. Fix: alpha 0.6 edge + 0.08 wash; hover 0.18. S.
- Focused composer: board = border glow(38%) + `0 0 20px accent@10%`. Code: always composer_border_color (2x line contrast, design.rs:173) and merely tints fill 10% when pane active (chat.rs:5515). Add focus-within glow edge + shadow. S.
- Composer surface: board bg dark inset rgba(10,20,28,.7) w/ line-2 edge, r10. Code r8 (radius::CONTROL), bg raised_background(card+tint). Use fill(1) over pane + hairline(1); r10. S.
- Branch row: board is inside the card, 32px, ink@5% fill, 1px hairline below, mono 11 tx-2, branch glyph accent (14px), "42 files changed" right. Code: separate strip above (composer_git.rs:815-850) with rounded 7px, border composer_border_color, label line spacing from text_size, `change_bar` rail. Structure differs by design (docked git strip); align height to 32, fill ink@5%, accent branch icon. M (composer_git.rs only).
- Context bar: board 4px, track tint@14% (rgba(130,200,230,.14) = wash level), fill --ok. Code 3px (chat.rs:5823 and composer_footer.rs:1459), track = surface_color(palette.line), fill = 3-stage ramp starting 0x3f9d6a (composer_footer.rs:1494-1530). Fix: 4px, track palette.wash(paper) style 14% tint, keep ramp but base from a status-ok constant. S.

## 4. Pane surface (agentpane board)  [M]  , src/chat.rs:6604, src/docking.rs:386-420, src/surface_blend.rs:150
- Board pane: 14px radius, linear gradient 180deg pane-hi -> pane-lo, i.e. tint wash 14% at top fading to 0 at 65% height, 1px glow(38%) edge, 0 0 32px glow@8% outer, 1px top sheen ink@4%. Code: flat tinted_pane_background(paper, dark, 0.15) (chat.rs:6604, docking.rs:533), contour radius 8 (docking.rs:397), edge colour = appearance.active_border_color (configurable, fine), glow only while working.
- Fix: paint a top linear_gradient(180, palette.wash(base) -> base at 0.65) quad under content (design.rs:wash exists; use gpui `linear_gradient` as in activity_sparkles.rs:603). Treat radius 14 as an option (Tern tiles use 8 by design setting; do not change silently). Running state currently paints an accent wash rising from the BOTTOM (activity_sparkles.rs:603-615: clear at 0, wash at 1.) , opposite to the board's top wash; consider moving it to the top. M.
- Tiling: 8px gap over --window: matches design::tern_spacing; no change.

## 5. Turn row  [S]  , src/run_activity.rs:1508-1600
Board: 28px high button, pad 0 8, r8, 12px label tx-3 (default), hover fill-3 + tx-2, 12px chevron, right mono 11 timestamp, failure count text in err colour ("1 failed" coloured). Code: Button.text().compact().xsmall().opacity(0.6) (:1580-1586), label 10px, brain icon (activity_icon) instead of chevron, whole summary one string incl. " · N failed" uncoloured (:1527), time child appended only for reasoning (reasoning_time). Fix: size 12px, replace opacity with palette.text(2) color, hover bg fill(2), split failed span with status err colour, always show mono time right. S-M (single file, but the label is a single String; needs spans).

## 6. Status pill  [S]  , src/working_bubble.rs:195-345
Board: 26px high, pad 0 10, pill r999, bg base@90%, 1px line-2, 12px tx-2 (plan prefix tx-1), 12px spinner accent stroke 2, stop = 9x9 r2 outline 1.5px tx-3. Code: 11px text, py3 px7 (~22px), border palette.line (level 1 not 2), text palette.ink, Spinner xsmall accent, Stop = 18x18 ghost Button with Square icon opacity .5, includes Work Plan toggle. Fix: 12px, h 26, hairline(1) edge, text(1)/(0) colours, stop glyph as 9px outlined square; failed variant (err@40% edge, X icon, "Failed", tx-1) is marked proposed , add when a run fails (needs state in WorkingBubble). S + optional M.

## 7. Dock strip  [M]  , src/collaboration.rs:1248-1330
Board: footer second row pad 8/12, buttons 28px r8 13px, 14px icons, tx-3 default, hover fill-3 + tx-1; entries: Subagents (n), Agent map | right: Work Plan, Build Queue (n); border-top line-1. Code: bar 34px (PANE_STATUS_BAR_HEIGHT collaboration.rs:28), px 10, gap 5, buttons text size `font_size-4` (~10px), r4, hover = accent border + accent@10% wash (:1257-1275, selected too). No Agent map button here and Work Plan lives in the working-bubble pill instead (working_bubble.rs:276). Fix: 28px buttons r8, 13px, hover fill(2/3) with ink text, selected = fill(3)+hairline(1), accent only for count? Add Agent map (agent_graph.rs) and Work Plan entries or accept the divergence. M.

## 8. Messages  [S]  , src/chat.rs:4560-4760 and 4890-4970
- Name 13/600 + mono 11 timestamp tx-3: code name 11px*ratio semibold, timestamp mono 10px muted (4557, 4755). Raise to 13/11 only if the transcript font scale allows; keep ratio.
- Avatar 22px r6 with agent->agent-deep 135deg gradient: code 21px rounded_md, provider logo or flat colour (4574-4640); give the fallback tile an accent gradient (palette.accent -> mix toward ink 25%). Timestamp separator is " - " (chat_time_label chat.rs:437) vs board " · ". S, trivial.
- Chat body is plain on pane (board): code gives user/auxiliary a bubble with border+raised (OK, not on board) but final answers get FINAL_ANSWER_GREEN-tinted border + background (chat.rs:~4896-4915); board's structured-report = raised card: fill-2, line-1, r12, pad 16, gap 12, title 16/600, inline code chip mono 12 pad 1/5 r4 line-1, "Sources used" list mono 11 tx-2. Fix: swap the hardcoded green for neutral fill(1)/hairline(0) (hard-coded colour breaks the theme-only rule). S.

## 9. Footer row  [S]  , src/composer_footer.rs:910-1060
Board: 40px tall, pad 0 16, 12px tx-2 label + 14px shield; right: JEV badge (mono 10, pad 1/7, pill, 1px accent@55%), brain icon 14 tx-2, model tx-2 + effort tx-3. Code: px TIGHT(5) pt 5, text 10-11px, hover fill palette.soft, access button py5 px7. Fix: pad 16, 12px, JEV badge pill with accent@55% edge (check whether badge exists , grep "JEV" in composer_footer*.rs before editing), model/effort tx-2/tx-3. S.

## 10. Tooltip / popups  [S-M]
Board tooltip: pad 6/10, r8, 12px tx-1, 1px line-2, solid raised bg, shadow 0 6 20 black@40%, 8px gap, never covers neighbour (flip). Code vendor tooltip.rs:116-121: popover blur bg, border theme.border (palette.line, level 1), shadow_md, r theme.radius(8). Fix: border -> hairline(1); shadow params; keep collision flipping logic check (tooltip_delay_tests exist). Popup menu row 32px r6 + hover fill-3 (board) vs vendor popover_style/popup_menu (not inspected, M).

## 11. Sparkle field  [S]  , src/activity_sparkles.rs:48-110
Board: 4-8 four-point stars, 6-26px, accent, 2.2-4s staggered twinkle (opacity .2->1, scale .75->1), stops when idle, pane gets glow(38%)+0 0 24 accent@8% while running. Code: 21 stars sizes 7-13, single CYCLE 3.4s (activity_sparkles.rs:48-72), random placement. Fix: reduce count to 6, widen size range to 6-26, per-star durations 2.2-4.0, lower base opacity .2. S.

## Not verified / needs follow-up
- Switch and radio dimensions in vendor (switch.rs/radio.rs) not measured against 36x20 / 18px.
- Workspace tab strip (board: 32px tab, r8, 12/500, 14px r4 colour chip, active fill-4 + line-2): the sidebar tabs use a different design (SIDEBAR_TAB_RADIUS 5, 25px height, 9px caps, design.rs:352-367) and a favicon-style icon, not a colour chip; board vs sidebar mismatch is probably an overview-board decision, not covered here.
- Searchable select (forms board) component existence not confirmed (palette_search.rs / palette_catalog.rs only seen by name).
- Mobile (Svelte) density table in impl board is out of scope.
