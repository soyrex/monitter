# GPUI animation plan

Status: proposal, 12 October 2026. Worktree: `/Users/alex/code/monitter-gpui-motion-plan`. Branch: `plan/gpui-motion-20261012`. Source snapshot: `7578bcbd91dd3689b138c88314d2eb9294e193f2`, including the main checkout's existing edits without changing its files or index.

## Recommended direction

Add brief feedback around actions, including a smooth slide when the main sidebar changes size or visibility. Start with that sidebar transition, workspace selection, Copy confirmation and attachment arrival. These should make everyday interactions feel more considered while keeping the workspace calm.

The live window was visually inspected: its three conversation panes, running-task sparkles, status indicators and compact controls already provide considerable activity. The strongest opportunities are the small moments when an action changes state. Preserve the existing accent colours, fine borders and compact spacing; give movement a short ease-out with no bounce or overshoot.

The live inspection establishes visual context; the source findings below refer to the frozen worktree snapshot. This is a native GPUI proposal. The older [interface motion proposal](INTERFACE-MOTION-PLAN.md) records web-client work and is not evidence of native implementation or performance.

## Proposed animations

These timings are design starting points, to be tuned in a native preview. They are not measured performance results.

| Priority | Interaction | Proposed appearance | Duration | Implementation boundary |
| --- | --- | --- | --- | --- |
| First | Main sidebar size and visibility | The sidebar edge slides between expanded, compact and, where supported, hidden positions. Adjacent workspace space changes continuously with it. | 180–220 ms | Preserve retained pane identities and focus. Reverse from the current width on another toggle; direct divider dragging follows the pointer immediately. |
| First | Workspace selection | The selected pill's accent tint and border ease to their final values. Selection text and content update immediately. | 120–150 ms | Keep pill width, focus outline, pane content and native browser bounds fixed. Retarget rapid selections. |
| First | Copy confirmation | The Copy icon fades into a checkmark, holds briefly, then returns. Feedback stays beside the copied message. | 100 ms change; 800 ms hold | Begin after the clipboard write. Use a stable message ID and one cancelable expiry; repeated Copy renews it. Keep the hit target unchanged. |
| First | Attachment arrival | A newly ready chip fades in while settling upward by 3 design pixels. | 120 ms | Animate only after storage succeeds. Allocate final chip geometry once, preserve the caret and draft, and keep removal immediate initially. |
| Next | Task completion | The existing status marker resolves into its final checkmark with a soft fade. The persistent Done label remains. | 160–180 ms | Trigger once for an observed successful completion. Do not play on history load, reconnect, row recycling or completion while hidden. Errors and cancellations retain distinct states. |
| Next | User-opened details | The chevron makes a quarter-turn; newly disclosed content has a short opacity entrance. | 100–140 ms | Commit final layout once. Avoid animated row heights or repeated transcript measurement. Collapse and keyboard actions stay immediate. |
| Next | Preferences entrance | The card fades in with a 4-pixel rise, matching the feel of existing menus. | 140 ms | Activate the backdrop, focus trap and native-browser occlusion immediately. Animate the card only; preserve the existing blur. Start with entrance only. |

For Reduced Motion and Motion Off, show final states immediately. Copy still displays its static confirmation for the same hold period; completion still displays Done. No information depends on seeing movement or distinguishing colours.

## Main sidebar slide

Alex specifically requested motion for the main sidebar's size and visibility. Animate the actual sidebar edge and allocated width together so the workspace expands or contracts smoothly. Expanded-to-compact transitions slide to the existing icon-rail width; hide/show slides to or from the left edge where the current theme supports hiding. Restore the remembered expanded width on expansion. Sidebar labels can fade briefly as space becomes available, while icons and hit targets remain aligned with their painted positions.

Use an interruptible ease-out without bounce, initially 200 ms for the full transition. A second toggle reverses from the current on-screen position. Keep the sidebar mounted through its closing slide, then disable its hidden hit targets and accessibility descendants. If focus was inside a fully hidden sidebar, move it to the visible toggle control immediately. Do not add animated delay to direct divider dragging; the divider tracks the pointer, with a short settle only when an existing compact/expanded snap changes the target after release.

This transition affects layout and needs its own performance check. Retain the browser, terminals and chat entities while their available space changes. Apply at most one changed native-browser bounds update per frame, without hide/show or reattachment on each tick. Preserve composer focus, text selection and scroll anchors. Keep expensive transcript preparation independent of width interpolation, persist only the settled width, and prevent saved-width restoration or resize callbacks from fighting the animation. A live drag takes ownership immediately and cancels the tween.

The intended result is a visibly sliding sidebar with coordinated workspace movement. Establish that behavior in the isolated native preview, including a browser and active streams, before accepting its performance.

## Existing effects to retain and refine

Source inspection found sidebar/detail-tab content already slides 24 pixels over 160 ms; the companion rail has an interruptible transition of up to 240 ms. Back to bottom already scrolls over 280 ms. GPUI popovers enter with a 150 ms fade and 8-pixel movement, and native component dialogs also animate on entry.

Reuse these primitives. Once the first batch is reviewed, consider reducing sidebar travel and aligning popup distances with the proposed 3–4-pixel entrances. Existing effects should be tuned in context rather than given a second animation wrapper.

Keep the recent focus-highlight and browser fixes intact. The main-sidebar slide is the deliberate layout-animation exception: browser bounds change only as required by that actual layout change. Exclude new pane-wide pulses, independent pane resize animations, streaming-token entrances and repeated message-list staggering. Hidden panes must show their latest state immediately when revealed, without replaying missed transitions. Send acknowledgement is a possible later refinement, but it needs careful handling of rapid consecutive sends and the existing draft-clear logic.

## Shared motion policy and execution

Use the existing `native_motion` policy and retained GPUI entities. Store transition state in the smallest relevant control or status entity; key it by semantic identity and event generation. Cancel or retarget on a newer action. Stop frame requests when finished, hidden, removed or reduced motion becomes effective. A one-shot confirmation expiry may remain; it must not become a repeating render timer.

Keep decoding, image preparation and other expensive work on workers. GPUI/AppKit focus, layout and painting remain on their supported UI execution path. Additional threads alone will not make a per-frame transcript rebuild cheap. New animations should perform bounded interpolation and repaint their local surface without triggering history preparation, browser visibility changes or application-wide notifications.

Fix live system preference observation before adding motion. The current macOS bridge reads Reduce Motion at startup, but its follow function is empty and `native_motion` retains the value in a `OnceLock<bool>`. Replace that startup-only cache with live policy state, observing AppKit's accessibility display notification through `NSWorkspace.notificationCenter`. Preserve the existing explicit On override and System/Subtle/Off choices; ensure both GPUI's flag and activity effects use the updated value. Apple documents this notification mechanism in [accessibilityDisplayShouldReduceMotion](https://developer.apple.com/documentation/appkit/nsworkspace/accessibilitydisplayshouldreducemotion).

## Source entry points

| Area | Current native integration point |
| --- | --- |
| Shared policy and macOS observation | [native_motion.rs](../desktop-gpui/src/native_motion.rs), [reduce_motion.rs](../desktop-gpui/vendor/gpui-base/src/reduce_motion.rs) |
| Main sidebar slide | [main.rs](../desktop-gpui/src/main.rs), `sidebar_split`, visibility and resize ownership; [preferences_modal.rs](../desktop-gpui/src/preferences_modal.rs), `toggle_sidebar`; [sidebar_resize_fixture_tests.rs](../desktop-gpui/src/sidebar_resize_fixture_tests.rs), gesture and saved-width coverage |
| Workspace pill appearance | [workspace_tabs.rs](../desktop-gpui/src/workspace_tabs.rs), active pill rendering |
| Copy confirmation | [chat.rs](../desktop-gpui/src/chat.rs), `copy_message_text` and message action rendering |
| Attachment arrival | [chat_attachments.rs](../desktop-gpui/src/chat_attachments.rs), attachment completion and chip rendering |
| Completion marker | [collaboration.rs](../desktop-gpui/src/collaboration.rs), retained subagent status; [activity_effects.rs](../desktop-gpui/src/activity_effects.rs), existing running-effect exit |
| Details disclosure | [approval_prompt.rs](../desktop-gpui/src/approval_prompt.rs), technical-details expansion; keep approval controls immediate |
| Preferences card | [preferences_modal.rs](../desktop-gpui/src/preferences_modal.rs), existing focus and webview lifecycle |
| Reusable transitions | [sidebar_tabs_motion.rs](../desktop-gpui/src/sidebar_tabs_motion.rs), [popover.rs](../desktop-gpui/vendor/gpui-component/src/popover.rs), GPUI animation primitives |

## Delivery and acceptance

1. Establish an optimized native-build baseline and implement live Reduce Motion observation. Capture typing latency, frame gaps, CPU use and idle frame requests using the existing performance fixtures.
2. Prototype and profile the main-sidebar slide with a browser and active streams. Add workspace selection, Copy confirmation and attachment arrival as separate changes. Review them together for consistent timing and restraint before expanding the set.
3. Add completion, details and preferences effects only after the first batch passes the same checks. Keep modal exits and independent pane geometry animations outside this proposal.

Compare baseline, motion enabled and motion disabled in the same optimized build configuration with repeated, identical workloads: idle, multiple visible streaming panes, many hidden panes, long history, and a browser beside an actively typed composer. Record p50/p95 input latency, frame gaps, CPU and callback counts. Apple distinguishes input delays from rendering hitches; both matter here. See [Understanding user interface responsiveness](https://developer.apple.com/documentation/xcode/understanding-user-interface-responsiveness).

Acceptance requires no input regression beyond repeat-run variability, no extra hidden-pane animation callbacks, and no ongoing callbacks attributable to completed effects. Verify that local feedback animations do not repeat browser bounds/visibility operations or transcript preparation. For the sidebar slide, allow only actual changed bounds, coalesced once per frame; visibility and native-view attachment remain stable during the slide. Any measurable regression blocks that effect until its cause is resolved.

Exercise rapid navigation/reversal, switching away mid-animation, reconnection, IME typing, keyboard focus, Escape, repeated Copy, attachment failure/removal and success versus failure completion. For the main sidebar, cover expanded/compact/hidden transitions as supported by each theme, repeated reversals, drag takeover, remembered width, window resizing and hidden accessibility state. Review light/dark themes, Monitter/Tern styles, interface scaling and live OS Reduce Motion changes. Specifically reproduce typing beside the browser during sidebar movement and running-agent focus highlights to check for flicker.

Plan validation: live UI visual review and read-only source inventory completed. Implementation, native animation previews and performance measurements remain future work.
