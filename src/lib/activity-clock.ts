/** A single, visibility-aware clock for transient activity UI. */
type Listener = (now: number) => void;

const listeners = new Set<Listener>();
let timer: ReturnType<typeof setTimeout> | undefined;
let listeningForVisibility = false;

function documentIsVisible() {
  return typeof document === 'undefined' || document.visibilityState !== 'hidden';
}
function stop() { if (timer) clearTimeout(timer); timer = undefined; }
function notify() { const now = Date.now(); for (const listener of listeners) listener(now); }
function schedule() {
  stop();
  if (!listeners.size || !documentIsVisible()) return;
  // Align updates to elapsed-time boundaries and schedule only after each tick.
  timer = setTimeout(() => { timer = undefined; notify(); schedule(); }, Math.max(20, 1_000 - (Date.now() % 1_000)));
}
function visibilityChanged() {
  if (!documentIsVisible()) { stop(); return; }
  notify(); schedule();
}
function ensureVisibilityListener() {
  if (listeningForVisibility || typeof document === 'undefined') return;
  document.addEventListener('visibilitychange', visibilityChanged);
  listeningForVisibility = true;
}

/** Subscribe an active activity surface to the shared clock. */
export function observeActivityClock(listener: Listener) {
  ensureVisibilityListener();
  listeners.add(listener);
  if (documentIsVisible()) listener(Date.now());
  schedule();
  return () => { listeners.delete(listener); schedule(); };
}

/** Isolated hooks for regression checks; production UI only uses observeActivityClock. */
export const activityClockTesting = { listenerCount: () => listeners.size, tick: notify };
