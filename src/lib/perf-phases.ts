/** Synthetic diagnosis hooks; inert unless the current URL includes monitter-perf=1. */
const enabled = () => typeof window !== 'undefined' && new URLSearchParams(window.location.search).get('monitter-perf') === '1';
const MAX_START_AGE_MS = 5_000;
// Geometry probes are selected once at page load: the normal UI avoids both
// Performance API calls and repeated URL parsing in its measurement hot path.
const geometryEnabled = enabled();
type GeometryPhase = 'row-measure' | 'margin-measure' | 'footer-measure' | 'follow-layout' | 'sync-flush';

/** Synchronous duration only, not paint latency; nested phases overlap. */
export function perfGeometryStart(phase: GeometryPhase): (() => void) | undefined {
  if (!geometryEnabled) return undefined;
  const start = performance.now();
  return () => {
    const end = performance.now();
    const name = `monitter.geometry.${phase}`;
    try {
      performance.clearMeasures(name);
      performance.measure(name, { start, end });
    } catch { /* Diagnostics must never change rendering on unsupported engines. */ }
  };
}

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
