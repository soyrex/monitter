export interface BrowserViewportRect { x: number; y: number; width: number; height: number; visible: boolean }

/** Measure after DOM layout, and follow ancestor motion only while it is running. */
export function observeBrowserViewport(node: HTMLElement, onLayout: (rect: BrowserViewportRect) => void) {
  let active = false, unloaded = true, revision: string | number = '', lastReport = '';
  let frame = 0, destroyed = false;
  const ancestors: HTMLElement[] = [];
  for (let parent: HTMLElement | null = node; parent; parent = parent.parentElement) ancestors.push(parent);
  const refreshAncestors = () => {
    if (destroyed) return;
    const current: HTMLElement[] = [];
    for (let parent: HTMLElement | null = node; parent; parent = parent.parentElement) current.push(parent);
    if (current.length === ancestors.length && current.every((element, index) => element === ancestors[index])) return;
    observer.disconnect();
    ancestors.splice(0, ancestors.length, ...current);
    for (const ancestor of ancestors) observer.observe(ancestor);
  };
  const report = () => {
    refreshAncestors();
    const rect = node.getBoundingClientRect();
    const next = { x: Math.round(rect.x), y: Math.round(rect.y), width: Math.round(rect.width), height: Math.round(rect.height), visible: active && !unloaded && rect.width > 0 && rect.height > 0 };
    const key = `${window.innerWidth}:${window.innerHeight}:${revision}:${next.x}:${next.y}:${next.width}:${next.height}:${next.visible}`;
    if (key !== lastReport) { lastReport = key; onLayout(next); }
  };
  const moving = () => ancestors.some(element => element.getAnimations?.().some(animation => animation.playState === 'running' || animation.pending));
  const queue = () => {
    if (destroyed || frame) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      report();
      // WAAPI transform animations do not trigger ResizeObserver or
      // transitionend. Keep measuring until their final untransformed frame.
      if (active && !unloaded && moving()) queue();
    });
  };
  const observer = new ResizeObserver(queue);
  for (const ancestor of ancestors) observer.observe(ancestor);
  const signals = ['resize', 'scroll', 'transitionrun', 'transitionend', 'transitioncancel', 'animationstart', 'animationend', 'animationcancel', 'monitter:pane-geometry'];
  for (const signal of signals) window.addEventListener(signal, queue, true);
  return {
    update(nextActive: boolean, nextUnloaded: boolean, nextRevision: string | number) {
      active = nextActive; unloaded = nextUnloaded; revision = nextRevision;
      // Hides cannot wait for a background window's next animation frame.
      if (!active || unloaded) report();
      queue();
    },
    destroy() {
      destroyed = true;
      observer.disconnect();
      for (const signal of signals) window.removeEventListener(signal, queue, true);
      if (frame) cancelAnimationFrame(frame);
      active = false;
      report();
    },
  };
}
