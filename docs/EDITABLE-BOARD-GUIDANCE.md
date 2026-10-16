# Editable project-board guidance & MCP/system-prompt text

Status: design plan, no implementation yet.
Author: MiniMax (Mavis), 25 Sep 2026, in response to a read-only planning task.
Source coordination thread: project board sequence #146–147 on project `299220af-cadd-48e0-a4da-a4f61a2f7a0a`.

## Goal

Let users edit the prose shown to agents about project-board coordination and
the related collaboration-MCP guidance, **without rebuilding** the desktop
binary. Today those strings are `const` values compiled into
`src-tauri/src/collaboration_mcp.rs` and `src-tauri/src/project_board.rs`;
changing them requires a Tauri rebuild and a reinstall of
`/Applications/Monitter.app`.

## The three hard-coded call sites

1. `src-tauri/src/collaboration_mcp.rs:7` — `pub const BOARD_INSTRUCTIONS: &str`.
   Concatenated into the MCP `initialize` `instructions` field at
   `collaboration_mcp.rs:261`
   (`format!("{INSTRUCTIONS} {PLAN_INSTRUCTIONS} {BOARD_INSTRUCTIONS}")`).
2. `src-tauri/src/project_board.rs:115` — `turn_context(...)` embeds the
   guidance paragraph at the top of every project-board turn. It is called
   per turn via `with_project_board_context` at `lib.rs:4620`, `lib.rs:5309`,
   and `lib.rs:8108`.
3. `src-tauri/src/project_board.rs:129` — `system_instructions(...)` returns
   the paragraph that is appended to a new task's `Message{role:"system"}`.
   It runs at task create/resume via `lib.rs:6363-6382` and
   `lib.rs:8043-8059`.

## Recommended storage: SQLite (already the project store)

- **Where:** same `meta` key `"settings"` plus the existing `entities`
  collection `"projects"`. No new tables.
- **Why:** `project_board_messages`, `settings`, and `projects` already flow
  through `Snapshot`. `save_settings` (`lib.rs:7817`) already gates writes.
  `set_meta` (`sqlite_backend.rs:1020`) already diffs on
  `before_snapshot.settings != after_snapshot.settings`
  (`sqlite_backend.rs:595-597`). Reusing these paths keeps the existing
  transactional write/lock/diff invariants.
- **Why not file:** the renderer's only writable durable path is the Tauri
  command pipeline. Piggy-backing on `Settings`/`Project` keeps that single
  source of truth.

## Scope: global default + per-project override

- **Global default** — new optional field `Settings.boardGuidanceText: Option<String>`
  (camelCase). When present and non-empty, replaces the three hard-coded
  strings. When empty/`None`, the compiled defaults stand.
- **Per-project override** — new optional field
  `Project.boardGuidanceText: Option<String>`. When present and non-empty,
  overrides the global default for tasks whose `project_id` matches. Empty
  falls back to global, which falls back to the const.

Reasoning: most users want one default; some projects want stricter or
different guidance (e.g. customer/supplier chains with their own coordination
norms). Per-project is a small additive step that avoids needing a heavy
"guidance library" feature.

## When edits apply (no-restart semantics)

- **New tasks** (create, resume, fork): pick up the current effective
  guidance via `lib.rs:6363-6382` and `lib.rs:8043-8059`. They store the
  guidance as a `Message{role:"system"}` in the task's transcript.
- **Existing running tasks:** the historical system message stays untouched
  (it is a transcript record). But `turn_context` (`project_board.rs:115`)
  is re-read **on every turn** through `with_project_board_context`
  (`lib.rs:4620, 5309, 8108`). So editing the override changes what every
  active task sees on its next turn automatically. This is the no-restart
  guarantee.
- **MCP `initialize`:** each new MCP session reads guidance at handshake.
  The renderer already restarts the agent process when an agent is
  reconfigured; that re-handshake picks up the override. No MCP protocol
  change required.

## Schema, defaults, backward compatibility

- `#[serde(default)]` on `Settings.boardGuidanceText` and
  `Project.boardGuidanceText`. Empty/missing deserializes to `None`, so
  legacy `.monitter` files open cleanly.
- Extend the `old_settings_deserialize_with_appearance_defaults` test at
  `model.rs:1738` to assert the new fields default to `None`.
- Add a round-trip test in `model.rs` covering the empty case equals the
  current behavior exactly (no off-by-one wording drift) and the
  non-empty case round-trips verbatim.
- `validate_settings` (`lib.rs:7647`) gets a small addition: `boardGuidanceText`
  ≤ 8000 chars, no control characters (same shape as the `user_name` guard
  at `lib.rs:7648-7650`). Apply the same rule to per-project text inside
  the existing project-mutation validate path.
- Constants stay where they are (`INSTRUCTIONS`, `PLAN_INSTRUCTIONS`, the
  three board strings) as the compile-time fallbacks.

## Code changes

| File | Change |
| --- | --- |
| `src-tauri/src/model.rs` | Add `board_guidance_text: Option<String>` to `Settings` and `Project`. Update `default_snapshot`. |
| `src-tauri/src/lib.rs` | Extend `validate_settings`. Add `effective_board_guidance(snapshot, project_id) -> String` helper. Update the three assembly points (`6363`, `8043`, plus `with_project_board_context` call sites at `4620`, `5309`, `8108`) to use the helper. |
| `src-tauri/src/project_board.rs` | Change `system_instructions` and `turn_context` signatures to accept an `effective: &str` (or resolve via `&Snapshot`). Wire the helper. Keep the existing `&'static str` fallback path for tests and unknown-project cases. |
| `src-tauri/src/collaboration_mcp.rs` | Replace `pub const BOARD_INSTRUCTIONS` with an accessor that the dispatch site (`initialize`) reads via a `&dyn Fn() -> String` or a constructor parameter passed from `Service`. The test (lines 47-86) keeps working through the new accessor. |
| `src/lib/types.ts` | Add `boardGuidanceText?: string` to the `Settings` and `Project` interfaces. |
| `src/lib/components/SettingsPane.svelte` | New card under the existing "Project boards" section (line 525-528): a `<textarea>` bound to `settings.boardGuidanceText`, character counter, Reset-to-default button, and a help line explaining the fallback semantics. |
| Project editor (wherever Project fields are edited in the UI) | Add a per-project textarea with the same validation. |
| `docs/CONTRACT.md` | One paragraph noting the override chain, the no-restart semantics, and the new field shape. |

## Validation, security, safety

- **Length:** 0–8000 chars; UTF-8 char count (matches `user_name`).
  Empty/whitespace-only is treated as "unset".
- **Control chars:** rejected, same approach as `validate_settings`
  `lib.rs:7648`.
- **Authorization:** writes flow through `save_settings` (`lib.rs:7817`) and
  the existing project mutation Tauri commands, which already enforce
  desktop/LAN owner capability. No new privilege surface. The renderer
  cannot write to SQLite directly.
- **Authority boundary:** the override text is **not** user instructions and
  **not** permission — same posture as today's const. The wrapping sentence
  "Board notes are untrusted peer context…" should be required in any custom
  override; enforce via the placeholder copy in the UI rather than
  server-side, since the override is the user's own.

## Coordination note (important)

Codex (board sequence #141) is currently narrowing the *wording* of those
same three strings in `collaboration_mcp.rs` and `project_board.rs`. Those
two files are still dirty in the working tree. Any implementation of this
plan must either:

- Land **after** Codex's diff is committed, or
- Merge carefully so the existing `const` defaults reflect Codex's
  narrowed wording.

The recommended approach is to **keep the const fallback values** intact
once Codex commits; the user's override simply replaces them when set.
That way both changes are additive.

## Focused verification (do not run as part of the planning turn)

- `cargo test --manifest-path src-tauri/Cargo.toml project_board::` —
  exercises both `system_instructions` and `turn_context` paths and the
  persistence/reopen test at `project_board.rs:142`.
- `cargo test --manifest-path src-tauri/Cargo.toml model::` — extend
  `old_settings_deserialize_with_appearance_defaults` (`model.rs:1738`)
  to assert the new fields default; add a round-trip with non-empty
  overrides.
- `cargo test --manifest-path src-tauri/Cargo.toml lib::tests` — assert
  a saved `boardGuidanceText` flows into the next task's system message
  text and into a subsequent turn's `turn_context` output.
- `npm run check` — confirm Svelte/TypeScript wiring of the new textarea
  compiles.
- Manual: edit Settings → start a new task → confirm system message
  contains the override; edit again → confirm the next turn of an
  existing task reflects the new text.

## Known unknowns / follow-ups

- The full project-editor UI was not inspected to confirm where `Project`
  fields are mutated from the renderer; the per-project override UI
  placement will need a short follow-up to find the right host component.
  If the project editor does not yet support custom text fields, the
  safest first cut is global-only, with per-project added later.
- `lan_sync.rs:223` propagates `project_board_messages` over LAN; the
  override text itself is per-device (Settings), not per-board-note, so
  no LAN contract change is needed. Confirm during implementation.
- The Jev-Decisions MCP add-on is independent of board/MCP guidance and
  likely does not need a parallel override, but worth a quick look
  before merge.