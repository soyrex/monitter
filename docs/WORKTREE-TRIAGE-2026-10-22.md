# Clean unmerged worktree triage — 2026-10-22

## Scope and method

Snapshot against `main` at `7fb3a2e` on 2026-10-22. This report covers the 59 registered, clean, branch-backed worktrees whose branch tips are not ancestors of `main`. It excludes dirty worktrees, detached worktrees, branches already merged by ancestry, and unregistered directories.

For each entry, `ahead/behind` is the commit-count relationship from `git rev-list --left-right --count main...branch`. `patch + / -` is `git cherry main branch`: `+` means a commit patch not found as equivalent on `main`; `-` means a patch-equivalent commit was found. Patch equivalence and commit counts are triage signals, not proof that the whole feature is missing or present. Baseline/snapshot commits can appear unique, and later edits may make equivalent functionality differ textually. A source-level review is still needed before integration.

## Summary

- 59 clean branch worktrees are graph-unmerged; their latest commits range from 2026-09-13 to 2026-10-21.
- Large behind counts show these branches are based on substantially older `main` snapshots. Do not merge a whole branch as-is.
- Several feature families already have counterparts on current `main`—including OpenUI media, Idea List and Watch panes, Admin worker restart handling, agent worktrees, and Work Logs. Treat corresponding branches as sources for selective review, not as missing-feature proof.
- Most worth a focused, current-source comparison: persistent backend TUI (a distinct product expansion); mobile activity/Watch/Ideas parity; Ideas dock refinements; universal user questions; pane search; and Work Logs review fixes. This is a shortlist for review, not a merge approval.
- Do not integrate ACP Full Access inheritance without a dedicated security and permission-boundary review. Browser Basic Auth implementation exists on `main`, while its browser documentation still records end-to-end verification as outstanding.
- Several older branches are patch-equivalent or mixed-equivalence candidates; these are strong archive/supersession candidates after owner confirmation, not candidates for automatic deletion.

## Cleanup performed — 2026-10-22

After explicit user authorization, 16 clean worktrees were rechecked against the live worktree list and `main`. Every listed branch had zero unique patches in `git cherry main branch`, no merge commits, and a clean worktree. Their worktrees were removed with `git worktree remove`, then the matching local branch refs were deleted. The inventory table below remains the pre-cleanup snapshot; these 16 rows document the completed cleanup, not current worktrees.

| Removed worktree | Removed branch |
|---|---|
| `monitter-agy-acp` | `feature/agy-acp-agent-20260926` |
| `monitter-auto-hide-tabs` | `feature/auto-hide-pane-tabs` |
| `monitter-chat-output-scroll-bottom` | `feature/chat-output-scroll-bottom` |
| `monitter-harness-contract-20260926` | `goal/harness-channel-20260926` |
| `monitter-harness-history-20260926` | `goal/harness-history-20260926` |
| `monitter-harness-jev-eval-20260926` | `goal/harness-jev-eval-20260926` |
| `monitter-jev-model-status` | `fix/jev-acp-model-status-20260926` |
| `monitter-jev-route-visible-card` | `fix/jev-route-visible-card-20260926` |
| `monitter-laya-background` | `feature/laya-background-service` |
| `monitter-openui-native-images` | `feat/openui-native-images` |
| `monitter-outstanding-mobile-20260930` | `feature/outstanding-mobile-20260930` |
| `monitter-outstanding-native-20260930` | `feature/outstanding-native-20260930` |
| `monitter-thinking-history` | `fix/thinking-history` |
| `monitter-timeline-readable-activity` | `feature/timeline-readable-activity-20260926` |
| `monitter-ui-bridge-20260930` | `fix/ui-responsiveness-bridge-20260930` |
| `monitter-ui-composer-20260930` | `fix/ui-responsiveness-composer-20260930` |

## Branch inventory

`Triage signal` is intentionally mechanical: “unique” means at least one patch not found equivalent on `main`; “equivalent” means every patch was equivalent; “mixed” means both; “history-only” means no patch was counted either way (for example, merge-only history). It does not assert feature value.

| Worktree | Branch | Latest commit | Ahead / behind | Patch + / - | Triage signal |
|---|---|---:|---:|---:|---|
| `monitter-accounts-integration` | `integration/codex-accounts-20260918` | 2026-09-18 | 1 / 393 | 0 / 0 | history-only; verify feature coverage |
| `monitter-acp-full-access-inheritance` | `feature/acp-full-access-inheritance` | 2026-10-13 | 1 / 77 | 1 / 0 | unique; security review required |
| `monitter-acp-opencode-claude` | `fix/acp-opencode-claude` | 2026-09-16 | 1 / 423 | 1 / 0 | unique; old integration fix, verify current providers |
| `monitter-admin-provider-20261018` | `fix/admin-provider-refresh-20261018` | 2026-10-18 | 1 / 61 | 1 / 0 | unique patch; restart handling exists on main, compare narrowly |
| `monitter-agent-worktrees` | `feature/agent-pane-worktrees-20261015` | 2026-10-15 | 8 / 76 | 8 / 0 | unique patches; feature exists on main, review safeguards only |
| `monitter-agy-acp` | `feature/agy-acp-agent-20260926` | 2026-09-26 | 1 / 249 | 0 / 1 | patch-equivalent; likely superseded |
| `monitter-auto-hide-tabs` | `feature/auto-hide-pane-tabs` | 2026-09-14 | 1 / 564 | 0 / 1 | patch-equivalent; likely superseded |
| `monitter-auto-size-panes` | `feature/auto-size-panes-20261014` | 2026-10-14 | 2 / 76 | 2 / 0 | unique; compare current layout behavior before considering |
| `monitter-browser-auth` | `feature/browser-basic-auth` | 2026-10-14 | 2 / 76 | 2 / 0 | unique patches; implementation exists on main, verification gap remains |
| `monitter-chat-output-scroll-bottom` | `feature/chat-output-scroll-bottom` | 2026-09-14 | 1 / 565 | 0 / 1 | patch-equivalent; likely superseded |
| `monitter-chat-watcher-20261017` | `feature/chat-watcher-20261017` | 2026-10-18 | 4 / 61 | 4 / 0 | unique patches; Watcher exists on main, inspect only focused delta |
| `monitter-control-recovery` | `fix/quiet-recovery-queued-controls-20261015` | 2026-10-15 | 2 / 76 | 2 / 0 | unique; verify against current control recovery |
| `monitter-delegate-workplans-20261018` | `feature/delegate-workplans-20261018` | 2026-10-18 | 2 / 61 | 2 / 0 | unique; work-plan model/UI exists on main, compare semantics |
| `monitter-git-bar-task-cwd-20260930` | `fix/git-bar-task-cwd-20260930` | 2026-09-30 | 1 / 135 | 1 / 0 | unique; confirm task menu still lacks this action |
| `monitter-git-design-20261015` | `feature/git-design-20261015` | 2026-10-15 | 2 / 76 | 2 / 0 | unique; Git UI exists on main, likely design/refinement delta |
| `monitter-git-strip` | `fix/composer-git-strip-20261015` | 2026-10-15 | 6 / 76 | 6 / 0 | unique; inspect narrow fixture fixes, not snapshot |
| `monitter-harness-contract-20260926` | `goal/harness-channel-20260926` | 2026-09-26 | 1 / 215 | 0 / 1 | patch-equivalent; likely superseded |
| `monitter-harness-history-20260926` | `goal/harness-history-20260926` | 2026-09-26 | 1 / 234 | 0 / 1 | patch-equivalent; likely superseded |
| `monitter-harness-history-verify-20260926` | `goal/harness-history-verify-20260926` | 2026-09-26 | 4 / 224 | 1 / 3 | mixed; isolate the one unique verification change |
| `monitter-harness-jev-eval-20260926` | `goal/harness-jev-eval-20260926` | 2026-09-26 | 2 / 235 | 0 / 2 | patch-equivalent; likely superseded |
| `monitter-ideas-dock-pane` | `ideas-dock-pane-20261018` | 2026-10-19 | 7 / 49 | 7 / 0 | unique; Idea List exists on main, compare dock/move/split refinements |
| `monitter-interface-cleanup-20260924` | `feature/interface-cleanup-20260924` | 2026-09-25 | 3 / 278 | 3 / 0 | unique; old interface cleanup, low priority pending visual review |
| `monitter-ios-companion-20260929` | `feature/ios-companion-20260929` | 2026-09-29 | 9 / 155 | 8 / 0 | unique; assess against current mobile voice support |
| `monitter-jev-model-status` | `fix/jev-acp-model-status-20260926` | 2026-09-26 | 1 / 238 | 0 / 1 | patch-equivalent; likely superseded |
| `monitter-jev-route-visible-card` | `fix/jev-route-visible-card-20260926` | 2026-09-26 | 1 / 248 | 0 / 1 | patch-equivalent; likely superseded |
| `monitter-landing-page-draft-20260925` | `feature/landing-page-draft-20260925` | 2026-09-25 | 1 / 271 | 1 / 0 | unique; product/design decision before integration |
| `monitter-laya-background` | `feature/laya-background-service` | 2026-10-20 | 1 / 38 | 0 / 1 | patch-equivalent; likely superseded |
| `monitter-message-presentation` | `feature/peer-system-message-style-20261015` | 2026-10-15 | 2 / 76 | 2 / 0 | unique; compare with current transcript presentation |
| `monitter-mobile-interface-integration-20261019` | `feature/mobile-interface-integration-20261019` | 2026-10-19 | 4 / 48 | 4 / 0 | unique; high-value parity candidate; broad snapshot, extract focused changes |
| `monitter-mobile-interface-parity-20261019` | `feature/mobile-interface-parity-20261019` | 2026-10-19 | 1 / 48 | 1 / 0 | unique; activity notices/Watch/Ideas exist in part, verify actual gaps |
| `monitter-mobile-ondevice-speech` | `feature/mobile-ondevice-speech-20260928` | 2026-09-29 | 4 / 155 | 3 / 0 | unique; compare with current iOS/Android on-device speech |
| `monitter-openui-native-images` | `feat/openui-native-images` | 2026-10-21 | 1 / 5 | 0 / 1 | patch-equivalent; feature is on main |
| `monitter-outstanding-mobile-20260930` | `feature/outstanding-mobile-20260930` | 2026-09-30 | 2 / 135 | 0 / 2 | patch-equivalent; likely superseded |
| `monitter-outstanding-native-20260930` | `feature/outstanding-native-20260930` | 2026-09-30 | 3 / 135 | 0 / 3 | patch-equivalent; likely superseded |
| `monitter-outstanding-ui-20260930` | `feature/outstanding-ui-20260930` | 2026-09-30 | 10 / 135 | 4 / 6 | mixed; likely snapshot/rebase required, not whole-branch merge |
| `monitter-pane-search` | `feature/pane-content-search` | 2026-10-14 | 6 / 76 | 6 / 0 | unique; inspect current palette search and isolate missing behavior |
| `monitter-rename-all` | `feature/rename-all-20261015` | 2026-10-15 | 2 / 76 | 2 / 0 | unique; evaluate batch rename UX and safety |
| `monitter-sidebar-panel-fade` | `fix/sidebar-fixed-panel-fade-20260925` | 2026-09-25 | 1 / 272 | 1 / 0 | unique; low-risk visual polish, verify current layout first |
| `monitter-slash-build-context` | `fix/slash-build-context-20261015` | 2026-10-15 | 2 / 76 | 2 / 0 | unique; compare against current persisted build intent handling |
| `monitter-teaser-capture` | `feature/teaser-capture` | 2026-09-13 | 2 / 601 | 2 / 0 | unique; old LAN/access-code behavior, defer pending explicit product/security review |
| `monitter-thinking-history` | `fix/thinking-history` | 2026-09-22 | 1 / 314 | 0 / 1 | patch-equivalent; likely superseded |
| `monitter-timeline-readable-activity` | `feature/timeline-readable-activity-20260926` | 2026-09-26 | 1 / 232 | 0 / 1 | patch-equivalent; likely superseded |
| `monitter-tui` | `feat/persistent-backend-tui` | 2026-10-08 | 3 / 99 | 3 / 0 | distinct product expansion; consider separately, not a routine merge |
| `monitter-ui-bridge-20260930` | `fix/ui-responsiveness-bridge-20260930` | 2026-09-30 | 3 / 146 | 0 / 3 | patch-equivalent; likely superseded |
| `monitter-ui-composer-20260930` | `fix/ui-responsiveness-composer-20260930` | 2026-09-30 | 2 / 146 | 0 / 2 | patch-equivalent; likely superseded |
| `monitter-ui-menus-20260930` | `fix/ui-responsiveness-menus-20260930` | 2026-09-30 | 2 / 146 | 1 / 1 | mixed; isolate unique keystroke performance fix if still absent |
| `monitter-user-questions-20261018` | `feat/universal-user-questions-20261018` | 2026-10-18 | 2 / 58 | 2 / 0 | unique; high-value candidate, verify GPUI question flows and current backend |
| `monitter-work-log-plan` | `plan/agent-work-log-20261015` | 2026-10-15 | 2 / 96 | 2 / 0 | planning/docs branch; retain only if still useful as current operating guidance |
| `monitter-work-logs` | `feature/work-logs-20261015` | 2026-10-15 | 31 / 76 | 31 / 0 | major feature snapshot; Work Logs exist on main, do not merge wholesale |
| `monitter-work-logs-accounting` | `work-logs/accounting-20261015` | 2026-10-15 | 5 / 76 | 5 / 0 | parallel Work Logs tranche; compare current accounting implementation |
| `monitter-work-logs-accounting-final` | `work-logs/accounting-final-20261015` | 2026-10-15 | 12 / 76 | 12 / 0 | parallel Work Logs tranche; benchmark additions may be selective |
| `monitter-work-logs-context` | `work-logs/context-20261015` | 2026-10-15 | 3 / 76 | 3 / 0 | parallel Work Logs tranche; compare current context/access code |
| `monitter-work-logs-destination` | `work-logs/destination-review-fixes-20261015` | 2026-10-15 | 22 / 76 | 22 / 0 | parallel Work Logs review tranche; inspect cursor fix against current code |
| `monitter-work-logs-pause` | `work-logs/pause-review-fixes-20261015` | 2026-10-15 | 23 / 76 | 23 / 0 | parallel Work Logs review tranche; inspect anchor fix against current code |
| `monitter-work-logs-query` | `work-logs/query-export-20261015` | 2026-10-15 | 13 / 76 | 13 / 0 | parallel Work Logs query/export tranche; compare window clipping |
| `monitter-work-logs-retention` | `work-logs/retention-20261015` | 2026-10-15 | 11 / 76 | 11 / 0 | parallel Work Logs retention tranche; compare checked bounded compaction |
| `monitter-work-logs-retention-review` | `work-logs/retention-review-20261015` | 2026-10-15 | 15 / 76 | 15 / 0 | parallel Work Logs review tranche; compare safety checks |
| `monitter-work-logs-review` | `work-logs/storage-review-20261015` | 2026-10-15 | 7 / 76 | 7 / 0 | parallel Work Logs storage tranche; compare replay/open hardening |
| `monitter-work-logs-storage` | `work-logs/storage-20261015` | 2026-10-15 | 4 / 76 | 4 / 0 | parallel Work Logs storage tranche; compare current implementation |

## Recommended handling

1. For the 43 rows not listed in the cleanup table, refresh the inventory and assign a disposition. Do not remove unique, mixed, history-only, dirty, or detached worktrees without separate review.
2. First compare the TUI, mobile parity/integration, user-questions, Ideas dock, pane-search, and narrowly scoped Work Logs fixes against current `main`; for every candidate, review the actual diff and update/rebase in an isolated worktree before integration.
3. Treat branches with only patch-equivalent commits as likely superseded, but verify current tree content and any uncommitted state before removing their worktree or branch.
4. Defer ACP Full Access inheritance and LAN/access-code behavior for security/product review.
5. Record a disposition per worktree (integrate specific commits, superseded, retain active, or archive after owner confirmation) before cleanup.

## Limits

This is a branch/worktree triage snapshot, not a complete code review of all 59 feature deltas. The source comparisons in the companion audit were representative, and no tests/builds, merges, rebases, or cleanup were run for this report. Re-run the inventory before acting because worktree dirtiness, branch tips, and feature coverage can change.
