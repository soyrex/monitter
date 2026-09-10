# Workspace location and Git recovery

The desktop source for this change is `/Users/alex/code/monitter-desktop-worktrees/cross-agent-messaging`,
on `feature/cross-agent-messaging`. This is separate from the unrelated CME checkout.

The original desktop repository remains at `/Users/alex/Documents/Codex/2026-09-09/monitter`.
Its last recorded clean baseline is `abba54312e6cfa7adc6c381d73f7bbcee42b740c`
(tree `af168f544264ed97baea1cddf2429a47a93251d6`). iCloud evicted source files and loose Git
objects during this work, leaving a local clone stalled for more than 40 minutes. Source was recovered
outside Documents before implementation; no original source files or history were overwritten.

The incomplete clone metadata and hash-verified baseline recovery objects are retained under
`/Users/alex/code/monitter-desktop-worktrees/recovery`. The implementation is saved as a shallow Git
commit with the original baseline explicitly retained as its parent. Its full current source tree
is available locally; predecessor objects are unavailable until iCloud materializes them. This is
not an invented baseline or a claim that full history was copied successfully.

After the original folder is kept downloaded, fetch its `feature/macos-harness` history into this
checkout and remove the shallow boundary only after all predecessor objects are present and verified.
Do not reset the working tree or overwrite implementation while recovering history. The original
repository and the recovery source copies remain available for comparison.
