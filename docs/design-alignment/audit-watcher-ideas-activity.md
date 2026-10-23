# Audit 3: watcher / ideas / activity popover / activity overview (3 boards)

Code root: /Users/alex/code/monitter/desktop-gpui/src. Read-only; no builds. Line numbers are current HEAD.
Palette helpers (design.rs:401-438: fill/hairline/glow/wash/text/tint) are NOT used by chat_watcher.rs, ideas.rs or
activity_overview.rs. They use legacy p.line / p.soft / p.muted / p.ink / p.accent. Every "use X" below means the ladder helper.
Board ladders: fill 3.5/7/10/15, line 10/18, text 92/66/46/30, glow 38.

Cross-cutting (design.rs, assign to one owner, it collides with all three):
- C1 design.rs:752 status_color hardcodes red/green/amber (0xbd655b/0x4f8f63/0xb38b45). Board ok/warn/err are saturated
  (#34d399/#ffb04a/#ff5c6c) and used for dots, segs, borders and state words. Suggest Palette::ok()/warn()/err() derived
  from the theme (tone mix with accent hue), then status_color calls them. Medium, design.rs.
- C2 Hardcoded hexes that violate the "flow through palette" rule: chat_watcher.rs:1374 and :1588 (0xe2645a),
  activity_overview.rs:1543 (0x6594db comm line). Replace with C1 err() / a palette "agent" tone (mix accent toward blue,
  or p.accent). Small.
- C3 "Agent avatar": board uses a 135deg gradient rounded-square (r5-6; 16/18/20px) with a terminal glyph. Needs one shared
  helper deriving the gradient from p.accent (accent -> accent mixed 25% toward paper). Chat already has such an avatar; reuse
  that fn in ideas.rs/activity_overview.rs. Small, new shared fn (put in design.rs or an existing avatar module).

## 1. board-watcher.html -> chat_watcher.rs  (chat panes stay topbarless; ignore the board's pane header/Chat|Watch seg as a header)

Ranked by visible impact:

W1 [Large-ish, Medium effort] Activity feed type is tiny and structurally different. chat_watcher.rs:1911-1930.
  Board: 26px rows, gap 10; time mono 11 text(3) w34; TOOL NAME word (Command/Read/Edit/Search/Peer) 12px text(1) w64
  (newest row: row text(0), tool name = accent); target mono 11 truncated; newest row bright, rest text(2)/(3);
  Peer rows in "agent" colour, sans 12.
  Code: icon 9px + time at mono 7.6px (:1917), description 8px terminal font lh12 (:1927), colour palette.ink for all, gap 7,
  no tool-name column, no "newest bright". Unit tests assert these tiny sizes (:2539-2540 kind<=10.5, desc<=12.1) -> update.
  Fix: row h(26*ratio); cols time(34, mono 11, p.text(3)) | kind label(64, 12, text(1)/accent for first) | target (mono 11).
  Keep icon optional or drop (board has none). Colour peer rows via agent tone (C2).

W2 [Medium] Section headers. :1543-1550 label(): 11px SEMIBOLD sans, muted. Board: mono 11, letter-spacing .1em, text(2),
  uppercase; right-hand meta 12px sans text(2) (not mono). Code puts subagent counts in mono 10 (:1821) and has no meta for
  ACTIVITY ("Following live" + 6px accent dot) or WORK PLAN/NOW. Fix: label() -> FONT_MONO, 11*ratio, weight normal,
  tracking .1em (gpui letter_spacing if available), p.text(2); add right-meta slot. Subagents meta: "{n} running" in accent + " · {m} done".

W3 [Medium] NOW block lacks the board anatomy. :1648-1719. Code: only a 14px MEDIUM clamp-3 update text above a 2px live line,
  fixed h78 (:1656).
  Board order: NOW header + "Tool 5 of this turn"; command row (14px accent icon, "Running a command" 13, mono 12 chip with
  fill(0)-ish bg 5% r6 pad 3x8, elapsed mono 11 text(2) right); 2px live bar (track accent 16%: code 0x28 =15.7% OK, :1682);
  "LATEST UPDATE · 3:18 PM" mono 11 text(3)/(4); update text 13px lh1.55 text(0) with a 1px left rule hairline(1) +12px
  padding-left, clamp 4, trailing 6x13 accent caret (blink 1s steps). Fix: restructure now{}; caret = small accent div.
  Data: needs current tool name + elapsed from WatcherProjection (:132-199) / state.activity[0]. Medium.

W4 [Medium] Subagent rows undersized, status dot static. :1355-1436. Code py5 (~22-26px), name 11 MEDIUM, task 10 muted,
  time mono 9 (:1415), dot 6px static. Board: h30 (34 in pane board), gap 10, name 13/500 text(0), task 12 text(2) truncate,
  time mono 11 text(3)/(4), running dot accent with pulse (opacity 1->.35, 1.6s). Finished toggle (:1846-1855) is a
  ">" text button at opacity .5; board: 12px chevron icon + "13 finished" 12px text(2), h28. Error colour hardcoded (C2).
  Fix: bump sizes (13/12/11 x ratio), add `with_animation` pulse on active dots (respect cx.reduce_motion like :1639).

W5 [Medium] Plan card chrome and glow. :1733-1779. Code wraps the plan in a bordered r12 card with a black 0/2/8 shadow (:1746)
  and, when `working` (modal), an accent border at 0.8 alpha + pulsing 10-18px glow (:1768-1776).
  Board: plan is a flat section (pad 14x16, hairline(0) divider below), no inner card; the only glow is on the pane edge:
  border glow() (38%) + static 0 0 32px accent 8% + inset 1px top sheen (ink 4%).
  Fix: border -> p.glow() (replace mix_rgb(panel, accent, .45) at :1743 and rgb(accent).opacity(0.8) at :1768);
  idle border p.hairline(1); shadow accent@0.08 blur 32 (drop black shadow); keep pulse only if desired, cap alpha at .08-.12.
  Also plan segs/rows come from crate::plans::render_plan_popup_filling (plans.rs): check board spec there:
  segs h5 r3 gap4 (done=ok, active=accent, pending tint 22%), rows h30, 14px icons, right status word 12px coloured. (Other board owner.)

W6 [Medium] Floating controls ignore theme. :1956 and :1618 build `design::palette(&Value::Null, WindowAppearance::Dark)`;
  watcher_mode_button (:14-45) + build pill + interject bar use that dark palette, so light themes and custom accents get a dark pill
  with the default-theme cyan glow. Violates "colours flow through the theme palette". Fix: pass the live `palette`; border
  hairline(1); fill via surface_blend::floating_control_background(palette.panel, palette.dark).
  Board build/state pill: h22, r11, bg accent 14%, inset ring accent 38%, accent text 12px, 6px dot. Code pill h26*ratio (:1634),
  11px (:1638). Resize to 22 / 12px.

W7 [Small] Footer / actions. Board: h52 footer bg fill(0) (3%) with "Watching - read only" (eye icon + 12px text(2)) left, Pause (btn h30
  r8 12px border hairline(1)) and primary Interject (accent 14% fill, accent 38% border, accent text; message icon) right.
  Code has no footer: a floating centred pill "Chat | Steer" at bottom 8 (:2012-2048), 11px, rounded_full, with Steer popover.
  Topbarless constraint means keep the dock; but restyle: Steer as the primary (accent 14% fill + glow() border) and Chat as ghost.
  "Pause" has no backend equivalent in this file (new feature, skip unless wanted).

W8 [Small] Mode toggle look (:1438-1493). Board seg: padding 2, r8, bg fill(0)-ish 5%, border hairline(0), items h24 r6, 12px,
  icon 14; on-state = accent 14% fill + inset ring accent 38% + accent text. Code: single ghost "Watch" button (h24, px10, 11px icon)
  inside a rounded_full pill. Restyle container to r8 and active tint; keep position (no header).

W9 [Small] Layout rhythm. Body gap 20 / content px44 pt48 (:1558, :2101-2103); board section pad 14x16, hairline between sections,
  520px single column. Two-column wide layout (:1542) is an app extension; acceptable. For the narrow/modal path use
  14x16 section padding and hairline dividers instead of 12/20 gaps.

W10 [Small] "Following live" / "Jump to live" chip (board feed behaviour) absent. Add meta text with accent dot; chip appears when scroll paused.

Not gaps / already aligned: live bar 2px r1, 1.8s sweep (:1659-1698); line-clamp; build_claim/pill straddle; peer link to exchange.
Files: chat_watcher.rs (W1-W10), plans.rs (W5 note), design.rs (C1), tests inside chat_watcher.rs for W1.

## 2. board-ideas.html -> ideas.rs

Replace p.soft -> fill(1)/fill(0); p.line -> hairline(0)/(1); p.muted -> text(2); keep p.accent.

I1 [Medium] List row typography/spacing. :1155-1232. Board row: pad 12x16, border-top hairline(0), gap 12; title 14px/500 lh1.3
  truncate; excerpt 12px text(2) clamp1; meta row 12px, gap 10, mt2: author 12 text(1), tag 11, date mono 11 text(3)/(4) ml-auto.
  Code: pane base is 12px*scale (:382), title SEMIBOLD at that size (:1177), meta 10px (:1199), date non-mono, px_3 (12), py_3.
  Fix: title 14/MEDIUM, meta 12 (date FONT_MONO 11), px 16. Hover bg tint@5% (board rgba(130,200,230,.05)); code p.soft (:1168).
  Selected row (board .sel: bg accent 6% + 2px accent bar, 10px inset top/bottom) is missing: list/detail are an either/or swap
  (:536). If wide-pane split is not planned, skip; else add `.sel` styling.

I2 [Medium] Status icons are unicode glyphs in accent. :1126-1131, :1174. Board 14px stroke icons, mt2, coloured per status:
  open = circle text(2); in plan = ring + dot accent; done = check ok (stroke 1.8); dropped = dashed circle text(3).
  Fix: gpui_kit IconName (CircleDot/CircleCheck/Circle) tinted by status; ok via C1. Also in detail Status select (:710) use the icon (12px) + chevron icon, not "▾".
  Dropped styling: board dims only the title (text(2), strike colour text(3)) and avatar .6; code dims whole row to .55 opacity (:1163). Fix to title+avatar only.

I3 [Small] Avatars. :1145-1154 and detail :650-661. Board 18px (14px in filter chips). Human: round, bg fill(3) (~white 12%),
  initial 10px/600 text(0). Agent: r5 gradient (C3) + 10px terminal icon (not the "↗" glyph, and not flat accent). Code 16px.
  "agent" label in agent tone 11px beside author in detail (not in list). Updated-by shows a mini round avatar.

I4 [Medium] Capture field. :1022-1038. Board: margin 12/16/0, h40, r10, bg fill(0)-4% , border hairline(1), 13px text(2) placeholder,
  plus icon 14, trailing kbd chip "↵" (mono 11 text(3), border hairline(0), r4, px5). Code: h36, r8, bg p.soft, "＋" fullwidth glyph
  (:1030), plain 10px "↵" (:1035). Fix: dims + icon + chip div.

I5 [Small] Filter chips. :1049-1095. Board chips h24 r12 12px, px10, border hairline(0); on = fill(1) bg + hairline(1) border + text(0).
  Chips Mine/Agents carry a 14px avatar. Sort is a "Newest v" chip with chevron (code: static "Newest ↓" 10px text, :1089-1094; not interactive).
  Search toggle belongs in the header (board), code puts it in the filter row first (:1049). Code chips h23 10px (:1076-1079) -> 24 / 12.

I6 [Small] Group headers. :1114-1121: 9px muted px12 py8. Board: mono 11, tracking .1em, text(3), pad 8/16/4.

I7 [Medium] Header. :434-447. Code h36, title "Ideas · N" at 12px with project selector button "Project v" (unicode) and "⋯" glyph (:463).
  Board h48, "Ideas" 13/600 + count mono 11 text(2), icon buttons 28px r8 (search, more). Keep project selector (needed) but restyle
  as ghost chip; header h48 (pane chrome may be shared with dock; check gpui_kit::component::dock before changing height).

I8 [Medium] Detail view fidelity. :613-868.
  - Title: 18px BOLD h36 (:627-629) -> 20px/600 -0.015em lh1.25, pad 18/16/6.
  - Meta grid: label col 68 (:606) -> 84, row gap 10/8, value 13 text(1); dates mono 12 (code sans); pad 14/16 + hairline(0) bottom.
  - Source: board tag-chip with arrow-up-right icon, agent tone text + agent 10% bg, "Codex #12 · 3:18 pm" (:741-766 uses ghost accent button with task title).
  - Tags: 11px text(2) chip bg fill(0)-ish r4 px6 (code 10px p.soft rounded_sm); "+ add" text(3) 12 (code "+ Edit" button :796).
  - NOTE row: mono 11 .08em label (:823 plain muted); seg r8, pad 2, items h22 r6 12px, on = fill(2)... (code r6/r4 + p.soft :835-851).
    Save button is inline here; board saves via footer/auto. Keep but ghost.
  - Markdown preview (TextView::markdown :864): board .md 13px lh1.6 text(1), h4 14/600, inline code 12 mono bg 7% r4 px5, task checkboxes
    14px r4 (on = accent 14% fill + accent 38% border + check). Verify gpui_kit markdown theme; may need style overrides.
  - History: chevron icon + "History · 3 edits" 12 text(2) (code ghost button :871).
I9 [Small] Footer actions. :968-1017. Board footer h52, bg fill(0) 3%, border-top hairline(0), btns h30 r8 12px border hairline(1);
  primary "Send to agent" = arrow icon + chevron, accent text, accent 14% fill, accent 38% border; "Drop" ghost text(2).
  Code: h28 (:981,996), p_3 wrap, bg p.soft + outline (:983), no icons. And list footer (:1234-1241): board h40 fill(0) bg,
  "Stored in the project · 14 files" 12 text(2); code p_3 text "Stored in this project · Enter captures an idea" (drop the hint; count files).

I10 [Small] Pane surface: bg flat rgb(p.panel) (:379); board pane = wash gradient (wash(base) -> base by 65% height), r14,
  border hairline(1) or glow() when focused, inset top sheen ink 4%. Probably owned by dock/tile; coordinate before touching.

Files: ideas.rs only (+ C3 helper, possibly gpui_kit icons). Tests in ideas.rs reference debug selectors; keep them.

## 3. board-activitypop.html -> admin_activity.rs (the "Admin activity" popover; NOT activity_overview.rs)

Code is already structurally close (rail, dot, title/time, status line, clamp, Expand + Focus pane). Gaps:
A1 [Small] Width/header/padding. :264 w400 -> 440. Header :202 pad 13x11 -> 16/16/14, title 15/600 (code 12*? `font`=12px SEMIBOLD, :205),
   subtitle 12 text(2) (code font*0.82 ~9.8px, :206). Refresh icon button 32x32 r8 ghost hover fill(2) (code default compact).
A2 [Small] Entry type. Title 14/600 (code 12px), time mono 12 text(2) (code 9.6px sans, :238 -> use FONT_MONO), status line 12px, task name own line 12, body 13/1.5 text(1)
   clamp 3 (code 0.9*12 = 10.8px, no clamp: intro(...,180) char cut :243), actions 13px. Entry pad 14/16/12, rail 20px gap 12 (code 23px rail, gap10, pad 10x12).
A3 [Small] Ring colours. :222-234: dot bg alpha .10 and no ring. Board: 20px ring, fill 14%, 1px border 38% in state colour, icon 12 stroke 1.8.
   Code row bg tint .025 / hover .055 (:223-224): board rows are untinted, hover = fill(2); remove row tint. Rail line hairline(0) (code theme.border).
   Status word coloured, rest text(2); "failed" shows error first; "running" is the only pulsing one (LoaderCircle spin at :59 OK).
A4 [Small] Surface: popover_style(cx) (:264) - verify r12, border hairline(1), shadow 0 12 40 black 45%, NO glow. Colours are via theme.* (good, no hex).
   `theme.success/danger/warning` come from the gpui theme, check they track C1.
Files: admin_activity.rs only.

## 4. boards overview / overview1 / overview2 -> activity_overview.rs

(board-overview.html is the title slide: no UI to implement. overview1 = cluster map; overview2 = same with agent panel open.)

O1 [Large] Agent panel (overview2). Missing. Click on a card emits OpenTask (activity_overview.rs:1517) which navigates away.
  Board: right-docked 460px panel (top 56, border-left glow(), gradient pane, shadow -24 0 48 black 45%), header h48: 20px agent avatar,
  name 13/600, status pill (h22 r11, state tint 14% + 38% ring), mono elapsed, project chip (project colour 12% fill + 40% border, r6,
  7px swatch), close; body = NOW / WORK PLAN / SUBAGENTS / ACTIVITY sections (same as W1-W5). Open state dims everything else:
  non-selected tiles, hubs and washes at opacity .32; selected tile: border accent, ring 0 0 0 3px accent 22% + glow 36px accent 28%.
  Legend swaps right text to "Esc closes the panel - click another tile to switch". Needs: `selected: Option<String>` state, panel embedding
  ChatPane::render_watcher (chat_watcher.rs:1495, already has modal path begin_activity_watch_modal :781) via an entity handle, Esc handling
  (exists :2207). Files: activity_overview.rs + chat_watcher.rs + shell wiring that consumes OpenTask. Do last / separate owner.

O2 [Medium] Card (tile) spec. activity_overview.rs:1304-1526, consts :522-525 (CARD 199x104, HUB 144x60).
  Board tile 208x92, r12, pad 10x12, gap 7, bg wash gradient (pane-hi->pane-lo), border hairline(1).
  Code: r10 (:1442), pad 9 (:1441), gap 5, flat surface*0.96 (:1320-1324), border = STATE colour at .52 for every state (:1331-1336).
  Board borders: working = accent glow() + 0 0 24 accent 8% + 0 6 18 black 35%; needs-you warn 42%; failed err 42%; done hairline(1);
  idle hairline(0) + opacity .72. Code working glow = state colour .30, blur14 spread2 (:1452) - roughly 4x stronger than board.
  Rows: board 4 (head, segs, current line, meta); code 5 (title, progress, task title :1471-1481, metadata, updated :1494-1505).
  Board line 3 = mono 11 "current tool" (e.g. "> cargo test", ellipsis) or, for needs-you/done, sans 11 message
  (text(0) for needs-you question, red for failed). Board meta: "2 of 4 - 3 subagents" left (11 text(2)) and mono 11 elapsed right;
  needs-you shows "Reply" warn at right. Code shows task title + "N of M steps" + separate "updated" row.
  Fix: merge updated into meta row right (mono), replace row 3 with live line (needs per-task latest tool/question from snapshot; the
  same data WatcherProjection reads), restyle border/shadow, resize consts (CARD_W/H affect layout code :573-760, retest overlap tests).
O3 [Medium] "Quiet" tone mapping wrong. :1305-1314 greyscales ALL colours for Idle and Done. Board: Done keeps ok-green label and
  all-green segs; only Idle fades (opacity .72, border hairline(0), state text(2)). Fix: tone only for Idle; use .opacity(.72) instead of luma gray.
O4 [Medium] Segments + state indicator. :1337-1363, :1412-1425. Board segs h4 r2 gap3, pending = tint 22% (code h3, idle muted .28);
  active seg accent, error seg err (code coalesces to State colours; fine). State: 11px, 6px dot with pulse for working,
  nothing but coloured word for Done/Failed (code: 11px spinner for Working, 4px dot others, :1420-1423). Pulse dot per board; spinner optional (keep one).
  Agent chip: board 16px r5 gradient + glyph (C3); code 14px accent 18% tinted provider logo (:1383-1403): keep provider logo but use gradient bg? Product call.
O5 [Medium] Header bar. :1962-1991, filters :1847-1866, controls :1867-1961.
  Board: h56, pad 0x20, gap 16, bg window 92% + hairline(0) bottom. "Activity" 15/600; count mono 11 text(2) "17 agents - 4 projects"
  (code sans 12, "N chats - M projects - last 12h", :1983); 1px x 20 divider; filter chips h28 r14 12px border hairline(0),
  count in mono 11 text(3), selected = fill(1) + hairline(1) + text(0); Working/Needs you/Failed chips have a 6px state-coloured dot.
  Code: ghost compact Buttons with "Working 8" text, selected bg p.soft (:1860), no dot, no mono count, gap5, 13px.
  Search: board 200px h28 pill (r14) with search icon, "Find an agent", "/" kbd chip (code Input 205w, :1874; "/" handler exists :2201).
  View seg: ONE segmented [Clusters | List] with icons (code: separate ghost buttons for Projects, Workspaces, Clusters, List :1876-1928;
  the Projects/Workspaces grouping is not on the board: keep, but put as a second small seg or a menu). Zoom: board 28px bordered icon btns
  (minus/plus, r8) around mono 11 "100%" (w36); code "-" "+" text labels (:1934,:1955).
O6 [Small] Dot-grid backdrop. :2214-2228: 40px spacing, 1.5px dots, p.muted@22%. Board: 28px spacing, 1px dot, tint ink-tint 10%
  (hairline(0)-like alpha: p.tint().opacity(.10)). Match spacing/size/colour.
O7 [Medium] Project hub. :1744-1775 (consts HUB 144x60).
  Board 170x46 r12, pad 0x14, LEFT-aligned (code centre), bg project colour 10% (0x1a), border 1px project 40% (0x66)
  (code border_2 at .90 + bg .20 = far heavier, :1751-1753), name 13/600 with 8px r3 colour swatch (code 16px project icon),
  summary 11px text(2) with coloured fragments ("1 needs you" warn, "1 failed" err), wording "needs you" (code "need you", :1714) and "quiet".
  Code drops the spinner next to the name? board has none; remove has_working spinner (:1766) or keep as accent (product call).
  Project colour comes from data via parse_color (fine). Changing HUB_W/H shifts layout.
O8 [Medium] Cluster glow washes missing. Board: per project a 660px radial gradient, project colour 8% centre -> 3% at 38% -> 0 at 68%,
  behind tiles; dimmed to .32 when a panel is open. No radial primitive in GPUI quads: approximate with 4-5 concentric
  paint_quad circles (cheap) in the dot-grid canvas layer, or use gpui radial if gpui_kit exposes one. Layer under :2213 container.
O9 [Small] Legend/footer. :2132-2164: pad 15x8, 5px dots, font-2. Board h40, pad 0x20, gap 20, 6px dots (Working one pulses), 1px divider,
  text "Dashed line = agent is working" (+ code's comm line note is an extra, keep but drop "blue"), right text with kbd chip.
  Communication line colour hardcoded 0x6594db (:1543) -> palette (C2).
O10 [Small] Link strokes :1641,1672: 0.8px, state colour at .34/.18. Board: dashed accent for working only; others faint hairline(1).
  Match: idle/done/needs/failed edges = hairline(1); working = accent dashed.

Files for splitting without collisions:
 - Owner A: chat_watcher.rs (W1-W10), plans.rs touch only for W5 verification.
 - Owner B: ideas.rs (I1-I10).
 - Owner C: admin_activity.rs (A1-A4).
 - Owner D: activity_overview.rs (O2-O10), then O1 later with Owner A coordination (chat_watcher.rs + shell).
 - Owner E (first, small): design.rs C1/C3 additions (ok/warn/err, agent_avatar gradient) so A/B/D can consume them; do not edit design.rs elsewhere.

Priority by visible impact (top 10): W1 feed text 8px, O2 card border/shadow/rows, O3 Done greyed, O7 hub weight,
O5 header chips/dots, I1+I2 idea rows/icons, W6 dark-palette floating controls, O1 side panel, W3 NOW block, I8 detail typography.
