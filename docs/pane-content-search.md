# Cmd-K open-pane search

The native palette ranks matching panes in the current workspace first, followed
by matching panes saved in other workspaces, then the remaining catalog. Title
matches rank ahead of content matches within each group. Selecting another
workspace's pane restores that workspace and focuses the existing pane.

The retained palette owns a private in-memory SQLite FTS5 index. Catalog refreshes
update only changed documents and remove absent entries. Keystrokes query the
index with escaped, parameterized tokens; they do not fetch transcripts, inspect
terminal screens, run browser scripts or rebuild the index. Prefix and bounded
typo matching use a vocabulary capped at 5,000 terms (4–24 characters). Results
include a short matching excerpt. No search document is written to disk.

Snapshots refresh on palette opening. Chat search covers retained message text;
terminal search covers retained output and scrollback; Neovim covers rendered
grid text and filename metadata, not unloaded file contents. Browser page text
arrives asynchronously from innerText, without reading input value properties.
Each pane snapshot is capped at 512 KiB, retaining recent chat/terminal text.
Unloaded or unavailable content remains searchable by title/metadata. Browser
callbacks are discarded after closing/reopening the palette or changing browser
metadata. Unsaved composer drafts are not indexed.

Validation: command_palette tests cover ranking, incremental FTS changes/removal,
Unicode, typo matching, mixed title/body queries, category filters, snippets and
query syntax. Build/check results are recorded in the Monitter build queue.

The native test harness requires GPUI test support:

```sh
node scripts/gpui-toolchain.mjs cargo test --manifest-path desktop-gpui/Cargo.toml --features gpui-kit/test-support --bin monitter-gpui-desktop command_palette:: -- --test-threads=1
```

Register and claim this command through the project build queue before compiling.
