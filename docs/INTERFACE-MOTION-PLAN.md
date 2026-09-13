# Interface motion proposal

Review baseline: `18771e7`. Branch: `feature/interface-motion-review`.
Status: implemented and merged into main on request; browser validation below.
Web-only delivery: no native rebuild is part of this change.

## Prototype

- Preview: `http://127.0.0.1:18464/` (normal dev remains on 18450).
- Appearance → Motion: System, Subtle, Off. This is local to the browser; OS
  reduced motion overrides Subtle. No backend settings schema is changed.
- Implemented control fades, directional sidebar swaps, settings/detail entrances,
  conversation and status fades, modal/palette entrances and guarded exits, menu
  entrances, brand/group reveals, deliberate tab scrolling and pane position motion.
- Pane dimming now covers the background and content without double-dimming.
- Empty split panes are pruned after their last tab closes or moves, including
  nested splits and runtime terminal removal. Recursive branch identity is retained
  during teardown, and temporary missing pane references no longer discard removal.
- Native userMessage/agentMessage lifecycle echoes are omitted from tool groups;
  their ordinary conversation bubbles remain the visible representation.
- Compact sidebar blades use a full-window 50% black/white blurred backdrop and
  stay above the pane header. Normal docked sidebars are unchanged.
- Large outgoing trees (over 512 descendant elements) skip the optional visual
  copy; incoming motion still runs. This bounds snapshot work on long sidebars.
- Sidebar width and pointer resizing remain immediate; terminal contents are not
  translated or remounted for animation. No streamed-token or history animations.
- To start this worktree reliably: `MONITTER_DEV_POLL=1 npm run dev:web -- --port 18464`.
  Polling is opt-in because native filesystem events were missed during this review.

## Validation record

- Chromium and WebKit focused action tests: motion policy, reduced-motion override,
  retained views, inert outgoing copies, message arrival eligibility and tab reveal.
- Chromium and WebKit pane tests: split/close/balance preserve the terminal identity;
  pointer resize creates no pane animation; theme dimming and crowded tab reveal pass.
- Both engines pass final chat/draft/settings/terminal tab closure, nested split
  movement and move-into-existing-pane collapse, with surviving panes filling space.
- Both engines pass blade backdrop coverage, theme/blur, header hit targeting and
  dismissal, including the mobile transformed-container layout.
- Chromium and WebKit UI matrix passes: sidebar reversal, composer draft/focus,
  active-tab hover, detail tabs, palette focus/close, local preference persistence,
  OS override, light/dark themes, three densities and narrow mobile layout.
- Both engines pass rapid dialog closure, Tab/Shift+Tab containment, reopening and
  keyboard-opener focus restoration with motion enabled and Off. WebKit required
  explicit palette focus cycling and restoring focus after the exit DOM flush.
- `npm run check`: zero errors and warnings. `git diff --check`: clean.
- Live preview loads the existing workspace through the backend. Test data above
  is browser-fixture-only; QA did not send messages or create real terminals.
- 200-task Chromium stress comparison initially measured p95 frame gaps of 33.4ms
  Off versus 66.4ms Subtle. This prompted the bounded outgoing-copy rule above.
  The post-limit run had zero ghosts/errors, but both modes slowed substantially
  under host contention (148.4ms Off / 134.1ms Subtle p95), so it does not establish
  a performance gain. Production profiling is still needed before claiming smooth
  frame rates for very large lists. The scripts retain this check for reproduction.

The original review and design rationale follow.

## Direction

Quiet, quick and interruptible. Use fades for changes in emphasis, short slides
for navigation, and geometry movement only when an object genuinely changes place.
No bounce, elastic easing, repeated list staggering or full-screen desktop swipes.
Selection, keyboard focus and application state update immediately; motion never
delays an action or a backend request.

## What exists today

- AppSurface mobile sidebar/chat navigation already slides a full viewport in
  240ms and respects reduced motion. Keep this spatial model on mobile.
- Tab status/close controls fade in 120ms; compact tab picker uses a 160ms
  fade/6px translation; slash suggestions enter in 160ms.
- Compact right detail panel has a 180ms entrance, but desktop panel and internal
  detail-tab changes lack a shared entry/exit treatment.
- PaneGrid lowers opacity and applies grayscale over 140ms. This reveals the
  same parent background, explaining why the background does not visibly dim.
- Sidebar modes switch conditional content immediately. setSidebarView deliberately
  updates local UI before any workspace restoration; preserve that responsiveness.
- Settings switches have 150ms transitions, but category changes are abrupt.
- Command palette and shared modal have visual depth but no common open/close motion.
- Active-tab reveal currently sets scrollLeft immediately, including after resizing.

These findings are from source inspection, not a frame-rate benchmark or a claim
that new animations have been visually validated.

## Proposed treatments

| Surface / action | Treatment | Timing |
| --- | --- | --- |
| Buttons, icon tiles, sidebar rows | Interpolate background, border and colour; fade optional controls. Do not fade primary text or move the click target. | 100–140ms |
| Active tab hover | No new background/colour animation. Keep the existing close-button reveal. | 120ms reveal |
| Sidebar mode: Agents → Activity → Projects | Incoming content starts 16px to the right and fades into place; reverse when moving back through the icon order. Keep logo, mode buttons and footer stationary. No per-row stagger. | 180ms entry, 100ms exit |
| Settings category / right-sidebar tab | Fade content with a 4–6px entrance. Keep navigation controls fixed; no full panel sweep. | 140–160ms |
| Chat or channel tab selection | Brief incoming content fade from 0.85 to 1; optionally 4px translation only after testing. Do not crossfade two readable conversations or animate the composer focus. | 120ms |
| Terminal tab selection | No content translation or crossfade. Keep cursor and terminal drawing crisp; animate tab chrome only. | 100–120ms |
| Search / command palette / dialogs | Fade backdrop; panel rises 6px while fading in. Exit slightly faster. Preserve current blur and black shadow. | 160–180ms entry, 100–120ms exit |
| Menus / suggestions | Standardise on the existing 6px rise/fade. Keep keyboard highlight immediate. | 120–160ms |
| Right detail panel open/close | Enter from the right by 12px with a fade. Desktop layout snaps to final geometry first; avoid repeatedly resizing the chat/terminal during the visual transition. | 180ms |
| Sidebar collapse / expansion | Crossfade wordmark and m, plus a small content reveal. First pass changes column width immediately; consider width animation only after terminal and layout profiling. | 160–180ms |
| Agent/project group expand | Short clipped reveal and chevron rotation for user-triggered expand/collapse. Do not animate live background list updates. | 140–160ms |
| Sending → sent / thinking → response | Fade status in place; keep message content stable. Only newly inserted messages may use a 4px/120ms entrance. Never replay on streamed tokens, history load or reconnect. | 100–120ms |
| Active-tab auto-scroll | Smooth only for deliberate tab selection, short and interruptible. Resize and drag reconciliation stay immediate. Preserve edge fades and full active-tab visibility. | ~160ms |
| Pane split / close / balance | Later phase: animate final visual positions, not live drag or backend state. No motion while resizing with a pointer. | 180–220ms |

For the sidebar, direction follows the displayed icon order rather than always
sliding left. Clicking a later mode brings it in from the right and moves the old
view left; choosing an earlier mode reverses this. Activity can also restore a
workspace: animate only the sidebar transition, not two competing screen moves.

## Inactive-pane treatment (separate proposed fix)

Replace opacity-based dimming with an inert, pointer-transparent overlay over an
opaque pane surface. Dark mode blends toward black; light mode toward a neutral
light surface. This affects background and content together. Avoid stacking the
overlay with the existing opacity reduction. Preserve the setting's direction:
a lower inactive-pane opacity means stronger deemphasis. Validate mapping before
changing its description; do not silently invert the slider.

## Implementation boundaries

- Start with shared CSS duration/easing tokens, explicit transitioned properties,
  and a small local animation helper. Use CSS for hover states; use interruptible
  Web Animations or local Svelte transitions for view entry/exit where appropriate.
- Do not wrap AppSurface, PaneGrid or terminal views in a global keyed transition.
  Their mount/unmount paths manage pane state, terminal attachment and input focus.
- Retained outgoing content must be inert and aria-hidden. Never leave two active
  composers, focus traps, live regions or terminal hosts during a crossfade.
- Modal and palette cleanup currently restores prior focus on unmount. Start with
  entrance-only animation; add exit motion only with an explicit closing state and
  verified focus containment/restoration. Settings category changes also clean up
  remote-control targets, so never delay that cleanup just for an outgoing fade.
- Preserve each view's scroll position, drafts, selection, caret and keyboard focus.
  Search must focus immediately; Escape/backdrop close must work during animation.
- New navigation cancels an in-flight transition instead of queuing it. Clean up
  animations on destroy and skip them for initial hydration/background snapshots.
- No CSS `transition: all`, animated blur/filter, full-window snapshots or global
  smooth scrolling. Avoid large permanently composited layers and permanent will-change.
- Terminal runtime debounces ResizeObserver refits by 80ms. Geometry animation
  needs explicit testing for resize storms, redraw and cursor/focus loss.
- Keep selectors locally bounded: AppSurface's recursive structure already makes
  some dev CSS transformations costly. Motion must not worsen startup/HMR delays.

## Accessibility and settings

- Honour prefers-reduced-motion everywhere: remove translations/scales and use
  immediate changes or a very short opacity change. Preserve progress meaning.
- Proposed Appearance control: Motion → System (default), Subtle, Off. System
  uses Subtle unless the OS requests reduced motion. Keep the OS preference as a
  hard limit even when Subtle is selected. Off disables decorative transitions.
- Keep this client-local, like tint, so desktop and a slower mobile client can
  differ. No backend schema or rebuild is required for a web prototype.
- Focus outlines and urgent errors appear immediately. No animated dismissal of
  unresolved errors or approvals; motion does not change notification lifetimes.

## Delivery order and isolated preview

1. Foundation + control fades + modal/menu entrances + reduced-motion support.
   Add modal exits only after the closing-state focus tests pass.
2. Sidebar directional navigation + settings/detail fades. Review these in browser
   before extending motion elsewhere; they should deliver most of the polish.
3. Chat/status transitions and deliberate tab auto-scroll, with scroll/focus tests.
4. Optional pane geometry and sidebar-width motion only after lifecycle regression
   coverage and performance checks. Keep the separate nested-pane bug fix separate.

Worktree: `/Users/alex/code/monitter-interface-motion-review`.
Preview: `http://127.0.0.1:18464/`.
After dependencies are available in this worktree, run
`MONITTER_DEV_POLL=1 npm run dev:web -- --port 18464`; Vite's guarded web mode retains LAN/Tailscale
rules and derives allowed origins from the selected port. Reuse the existing
backend without rebuilding it. Do not replace the regular dev server on 18450 or
touch the in-progress native release checkout. Different origins isolate browser
storage, but the preview still points to real backend data: use fixtures for tests
and avoid creating chats, sending messages or changing shared settings during QA.

## Acceptance checks

- Chromium and WebKit; desktop and narrow mobile; light/dark; all three densities.
- Rapid repeated navigation and reversal settles on the last selection without
  blank content, accumulating listeners or stale animation completions.
- Active tab has no hover colour shift. Close controls remain accessible.
- Sidebar selection stays connection-specific and responds before workspace work.
- Composer focuses on tab switch without keyboard flicker or lost draft text.
- Terminal selection/resize preserves process, buffer, cursor and input; no duplicate
  terminal mounts. Pane close/drag regression tests remain green.
- Modal focus trap, immediate search typing, Escape and focus restoration all work.
- Streaming, reconnection and history loading never replay message animations or
  pull a reader away from older messages. Off-screen panes do not animate.
- Reduced motion and Motion Off retain every function without spatial movement.
- Profile long sidebar lists and multiple panes. Prefer transform/opacity, check
  paint/layout work and dropped frames; do not use animation to disguise latency.
