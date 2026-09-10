# Reference design view audit

Compared the supplied `Agent Harness Chat.dc.html` package with `design/reference.html` and the
Monitter implementation on 2026-09-09. Both HTML files are byte-identical. The reference contains
six static compositions; only its accent controls have implemented behavior. Decorative controls
are design proposals, not evidence that their underlying harness capabilities exist.

| Reference | Current implementation | Remaining view options |
| --- | --- | --- |
| 1a: nested sidebar | Agents with chats, plus Standard/Activity/Projects modes | The main nested layout is represented. |
| 1b: compact agent rail, chat tabs, run detail | Header chat tabs and a run detail pane | Compact agent rail and separate Run detail/Sub-agents tabs. Files, diff and token widgets would need reliable harness data. |
| 1f: sub-agents tab | Linked delegated tasks in run detail | Dedicated sub-agent tab and richer child-task presentation. |
| 1c: group channel | Local channels with explicitly selected recipients | Mention autocomplete and mention-based recipient selection. Multiplayer and iOS are deferred by the v1 scope. |
| 1d: conversational agent construction | Agent editor for instructions, harness, host, model, folder and permissions | In-chat construction, test-before-save and suggested adjustment chips. Tool/scope controls need harness-specific support. |
| 1e: task overview | Status groups, plus Activity sidebar ordering and Projects folders | Overview filtering, grouping by agent/project, and a distinct Needs you queue tied to actual actionable events. |

The most useful next view options are overview filtering/grouping, a compact agent rail, and
separate Run detail/Sub-agents tabs. Mention selection can improve local channels without adding
multiplayer. These are audit findings, not additions included in the current Projects/draft update.

The reference's decision buttons, files touched, diff previews and token meters must be grounded
in real runtime data before implementation. Monitter must not display fabricated progress or an
approval button its noninteractive harness transport cannot execute.

Reference sections: `design/reference.html` lines 34 (1a), 198 (1b), 316 (1f), 353 (1c),
458 (1d), and 593 (1e). Source paths: `src/routes/+page.svelte`, `src/lib/components/TaskActivity.svelte`,
`src/lib/types.ts`, `docs/CONTRACT.md`, and `docs/FUTURE_MULTIPLAYER.md`.
