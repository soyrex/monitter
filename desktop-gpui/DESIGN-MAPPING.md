# Native shell source mapping

This records source implementation, not screenshot or runtime parity verification.
The currently running preview is an older build; these edits do not replace it.

| Native surface | Existing source | Mapping |
| --- | --- | --- |
| Default light/dark colors | `src/lib/app-theme.ts`, `appThemes.monitter` | Exact paper/sidebar/panel/line/soft/code/ink/muted/accent base hex values in `design.rs` |
| Optional warm paper | `design/reference.html` | `#fbf8f2` paper, `#f2ede3` sidebar/user bubble, reference ink/muted; selected locally for this native session |
| Sidebar width | `AppSurface.svelte`, desktop grid rule | 252px default; resize gesture remains pending |
| Agent/chat hierarchy | `AppSurface.svelte`, agent groups/task tree | Real snapshot identities, collapse state, 32px avatars, nested chats, actual task-state dot; no seed conversations |
| Compact rows | `AppSurface.svelte`, side-scroll agent-row rules | 13px agent name, 10px provider/model, 12px chat label, compact controls |
| Chat header | `design/reference.html` | 58px row, 26px identity, 14px title, 11px workspace/status; provider/model opens actual model controls |
| Transcript | `TaskTranscript.svelte`, `design/reference.html` | Native virtualized Markdown; user bubbles align right, assistant content stays on paper; 13px default and saved chat-font size |
| Composer | `design/reference.html` | Outlined rounded panel with native multiline input and explicit send/stop; attachment/mention polish remains separate work |
| Fonts | `AppSurface.svelte`, SettingsPane font preferences | Respect installed saved interface family; choose installed IBM Plex Sans when available, otherwise native system fallback; no fictitious installed font |
| Component theme | GPUI component `Theme` tokens | Apply matching paper/ink/panel/line/muted/accent to controls and native Markdown |

The live Svelte default palette is neutral gray. The design reference uses warm
paper. Both are available explicitly; the native reference choice is not silently
written into the frontend's local-storage theme-pair format. The native accent
and light/dark/system preferences still use the existing backend settings.

Remaining style work includes configurable preset pairs/tint/contrast persistence,
sidebar resizing, pane grids/tabbars, rich status/usage chrome, Markdown code/table
actions, keyboard focus visuals, provider-logo/avatar images, and full component
appearance parity. Existing functional modules remain native; compact navigation
never advertises unavailable actions as working.
