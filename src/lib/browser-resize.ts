/** Suspend native overlays through a splitter drag or a burst of window resizes. */
export function guardBrowserResize(onSuspend: (suspended: boolean) => void) {
  let pointerId: number | null = null, windowResizing = false, suspended = false, destroyed = false;
  let settleFrame = 0, windowTimer: ReturnType<typeof setTimeout> | undefined;
  const suspend = () => {
    if (settleFrame) cancelAnimationFrame(settleFrame);
    settleFrame = 0;
    if (!suspended) { suspended = true; onSuspend(true); }
  };
  const settle = () => {
    if (destroyed || pointerId !== null || windowResizing) return;
    if (settleFrame) cancelAnimationFrame(settleFrame);
    // Let the final split ratio and ResizeObserver reports reach native bounds
    // before visibility is restored. A new resize cancels this reveal.
    settleFrame = requestAnimationFrame(() => {
      settleFrame = requestAnimationFrame(() => {
        settleFrame = 0;
        if (!destroyed && pointerId === null && !windowResizing && suspended) {
          suspended = false; onSuspend(false);
        }
      });
    });
  };
  const start = (event: PointerEvent) => {
    if (event.button !== 0 || !(event.target instanceof Element) || !event.target.closest('.pane-resizer, .sidebar-resize')) return;
    pointerId = event.pointerId; suspend();
  };
  const finish = (event: PointerEvent) => {
    if (event.pointerId !== pointerId) return;
    pointerId = null; settle();
  };
  const cancel = () => { pointerId = null; settle(); };
  const key = (event: KeyboardEvent) => {
    if (event.key === 'Escape') cancel();
    else if (['ArrowLeft','ArrowRight','ArrowUp','ArrowDown','Home','End'].includes(event.key)
      && event.target instanceof Element && event.target.closest('.pane-resizer, .sidebar-resize')) resize();
  };
  const resize = () => {
    windowResizing = true; suspend();
    clearTimeout(windowTimer);
    windowTimer = setTimeout(() => { windowResizing = false; settle(); }, 120);
  };
  window.addEventListener('pointerdown', start, true);
  window.addEventListener('pointerup', finish, true);
  window.addEventListener('pointercancel', finish, true);
  window.addEventListener('lostpointercapture', finish, true);
  window.addEventListener('blur', cancel);
  window.addEventListener('keydown', key, true);
  window.addEventListener('resize', resize);
  return () => {
    destroyed = true;
    if (settleFrame) cancelAnimationFrame(settleFrame);
    clearTimeout(windowTimer);
    window.removeEventListener('pointerdown', start, true);
    window.removeEventListener('pointerup', finish, true);
    window.removeEventListener('pointercancel', finish, true);
    window.removeEventListener('lostpointercapture', finish, true);
    window.removeEventListener('blur', cancel);
    window.removeEventListener('keydown', key, true);
    window.removeEventListener('resize', resize);
  };
}
