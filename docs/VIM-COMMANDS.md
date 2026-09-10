# Vim-style pane and tab commands

Monitter maps only commands that have a direct equivalent in its pane tree or
ordered tabs within the active pane. A Monitter tab is a chat, draft, channel,
terminal, or settings view; it is not a Vim tab page containing windows.

`tabnew`, `tabnext`/`tabprevious`, `tabfirst`/`tablast`, `tabclose`,
`tabonly`, and `tabmove` support Vim-style numeric, `+N`, `-N`, and `$` targets
where applicable. `q` and `quit` close the active Monitter tab.

`split`/`sp`, `vsplit`/`vs`, `close`, `only`, `resize`, and `vertical resize`
operate on panes. `:close` moves its open tabs to a surviving pane; `:only`
merges all panes into one. Neither abandons an active chat or silently stops a
run. The four-pane workspace maximum still applies.

`:wincmd h/j/k/l`, `t/b/w/W`, `s/v`, `c/o`, `r/R/x`, `H/J/K/L`, `=`, `_`,
`|`, `+`, `-`, `<`, and `>` parse as their matching Ctrl-W pane actions.
`parseVimWindowKey(key, count?)` is also exported for shortcut handling.

`completeVimCommand(text)` returns bare `value` plus a display `label` and
`detail`, for the command-line Tab key. File and buffer commands (`edit`,
`buffer`, `write`, `wq`, `bdelete`, and arbitrary Ex expressions) report
unsupported rather than pretending to manipulate Vim state.

The inventory follows Vim's [window help](https://vimhelp.org/windows.txt.html)
and [tab-page help](https://vimhelp.org/tabpage.txt.html), including their
navigation, resize, split, close, reorder, rotation, equalization, and count
forms where Monitter can represent them.

Choose **Standard** or **Vim** under Settings → Permissions & behaviour →
Keyboard shortcuts. In Vim mode, Escape then `:` opens the command line;
Tab and Shift-Tab cycle matching commands. Ctrl-W (or Cmd-W on macOS) arms
window commands without closing a tab. Terminal editing keys stay with the
shell while its input has focus.

Cmd-1 through Cmd-9 on macOS (Ctrl on other platforms) select visible tabs
within the focused pane, including the dashboard. Holding that modifier shows
the indexes. These shortcuts work in either mode. Vim tab commands address
chat, terminal and settings tabs, excluding the dashboard.

Resize counts approximate character columns and rows using the interface font;
ratios remain within the workspace's 15–85% limits. Pane rotation and exchange
move complete tab sets and drafts while retaining the existing split geometry.
