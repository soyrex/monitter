# UI responsiveness repairs — 2026-09-30

Audited from local main `532909f` with three Luna workers and an integration review. The focus was controls that wait on unrelated commands, menus that remain modal during saves, draft loss during asynchronous work, and synchronous work on interaction paths.

## Repairs

| Area | Observed cause | Result |
| --- | --- | --- |
| Controls palette | Preference actions awaited settings persistence before dismissing the palette and used the global busy flag. | Dismiss before saving; use a dedicated pending flag and visible success/error feedback. |
| Permissions picker | The picker awaited `setTaskSandbox` before closing. | Close immediately; show a pending indicator, and show failures without reopening or taking focus. |
| Task/channel composers | Unrelated foreground operations disabled send, attachment and voice controls. In-flight sends lacked a matching duplicate guard. | Scope submission guards to the current chat/draft; keep typing and other chats available. |
| New-chat creation | Routing/catalog lookup ran before capturing all context and clearing the submitted draft; later edits could be erased or another tab's context read. | Capture prompt, attachments, sandbox and destination before waiting; show the pending message immediately and preserve follow-up text through failures and tab switches. |
| New-chat keyboard focus | Entering or leaving the pending-chat view replaced the focused textarea. | Transfer focus to the replacement composer when it still belongs to the same interaction, preserving focus chosen elsewhere. |
| Transcript updates | Metadata-only updates rebuilt maps and sorted retained message arrays. | Preserve unchanged task/channel arrays, allowing downstream indexes to reuse them. |
| Typing and workspace persistence | Every composer edit serialized all retained drafts and attachment previews just to track dependencies, before the autosave debounce. | Track composer text separately; retain debounced capture and reload recovery. |
| Native account discovery | A synchronous Tauri command performed directory discovery while holding the shared state lock. | Use `spawn_blocking` and copy configured paths from the committed snapshot before filesystem work. The response and credential boundaries are unchanged. |

## Verification

- Production web assets: direct `vite build`, without the package script's LAN publication step.
- Svelte/TypeScript: `npm run check` reports 0 errors and 25 existing warnings.
- `scripts/ui-responsiveness-smoke.mjs`: WebKit and Chromium cover held/rejected task and channel sends, cross-chat sending during a pending stop, delayed routing/creation with tab switching, duplicate guards, failed creation with newer draft text, and keyboard focus across creation transitions.
- `scripts/ui-palette-responsiveness-smoke.mjs`: WebKit and Chromium pass delayed settings saves and visible errors after rejection.
- `scripts/access-picker-test.mjs`: isolated permission-picker success/failure checks pass in Chromium and WebKit, including focus retention after failure.
- `scripts/ui-sync-cache-test.mjs` and `scripts/snapshot-indexes-test.mjs`: no-op identity sharing and normal updates pass.
- `scripts/ui-workspace-typing-persistence-test.mjs`: WebKit and Chromium pass with 100 retained drafts and four attachment previews (about 2 MB). A 29-character input caused 29 additional broad dependency serializations before this fix and none after it; autosave and composer restoration after reload still work.
- Existing new-chat transition and advisory Jev first-message/route-confidence regressions pass. The transition test uses the sending tooltip because decorative animation characters make exact text matching inappropriate.
- The focused Rust account-discovery test passes after compiling the changed native command. Command-contract checks cover 115 native/68 LAN commands; all 7 negative drift tests pass.

The existing `scripts/ui-sync-history-test.mjs` fails its prepend-anchor assertion with an 832px shift. The identical failure reproduces on the baseline without the transcript-cache change. No scroll-anchor implementation was changed in this repair.

These checks exercise application behavior with a browser-only bridge fixture and the native account test. They do not establish a measured reduction in installed-app latency or prove that every possible freeze is resolved. No desktop app installation/restart or LAN publication is part of this repair.
