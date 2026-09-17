/** Synthetic diagnosis hooks; inert unless the current URL includes monitter-perf=1. */
const enabled = () => typeof window !== 'undefined' && new URLSearchParams(window.location.search).get('monitter-perf') === '1';
const MAX_START_AGE_MS = 5_000;
export function perfMark(name: string) {
  if (!enabled()) return;
  const mark = `monitter:${name}`;
  performance.clearMarks(mark);
  performance.mark(mark);
}
export function perfMeasure(name: string, start: string, end: string) {
  if (!enabled()) return;
  const measure = `monitter.${name}`;
  const startMark = `monitter:${start}`;
  const latest = performance.getEntriesByName(startMark, 'mark').at(-1);
  if (!latest || performance.now() - latest.startTime > MAX_START_AGE_MS) {
    performance.clearMarks(startMark);
    return;
  }
  performance.clearMeasures(measure);
  try {
    performance.measure(measure, startMark, `monitter:${end}`);
    // A mount is the terminal phase for this selection probe. The virtualizer
    // may report later in another retained pane; never attribute that to it.
    if (name.endsWith('open-to-mount')) performance.clearMarks(startMark);
  }
  catch { /* Initial mount has no preceding chat-selection mark. */ }
}
