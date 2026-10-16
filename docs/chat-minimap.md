# Native chat minimap

The transcript's right edge shows a 20px overview while browsing history. It
slides away when the transcript resumes following the latest message. Reduced
motion disables the transition.

- User messages use the interface accent colour.
- Final assistant answers use the same green as their chat bubbles.
- Bubble markers are at least 3px high and grow with message length, up to 16px.
- Bubble markers rest at 40% opacity and brighten to full opacity on hover.
- Other assistant messages use a 1px line at 20% opacity.
- Hovering a bubble grows its marker in width and height and opens an
  interactive, scrollable text preview after 500ms. The preview fades in and
  slides left over 180ms, settling to the left of the minimap.
- Clicking a marker animates the scroll to its message. Dragging the rail scrubs history;
  dragging to the bottom resumes following new messages.

Positions represent transcript row order, not estimated document pixels. The
virtual list does not need to measure every message. Dense histories share up to
192 marker bands; a band's hover preview retains all messages in that band.
Marker preparation occurs with transcript preparation, outside scroll callbacks.

## Isolated preview

Build with `npm run build:gpui -- --performance-profiling --visual-export`.
Launch the resulting executable with `MONITTER_GPUI_PERF_SCENARIO=minimap` and
`MONITTER_GPUI_PERF_HISTORY_ROWS=45`. This uses synthetic messages and does not
connect to a backend. Preview-only buttons move into history and return to the
latest message.

Focused test filters are `chat::minimap::tests`,
`chat::transcript_view::minimap_tests`, and `chat::transcript_view::tests`, using
the `visual-export,performance-profiling` features. Run the isolation tests with
`--test-threads=1` because their preparation counters are shared.
