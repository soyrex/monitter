import { motionEnabled } from '$lib/motion';

/** Fade only edges with hidden tabs; compact dropdowns must remain unmasked. */
export function tabStripFade(node: HTMLElement) {
  let frame = 0;
  let scrollFrame = 0;
  let revealPending: 'immediate' | 'smooth' | null = 'immediate';
  let selected: Element | null = node.querySelector('.tab-entry.active');
  let deliberateSelection = 0;
  const cancelScroll = () => {
    if (scrollFrame) cancelAnimationFrame(scrollFrame);
    scrollFrame = 0;
  };
  const scrollTo = (target: number, smooth: boolean) => {
    cancelScroll();
    if (!smooth || !motionEnabled() || Math.abs(target - node.scrollLeft) < 1) {
      node.scrollLeft = target;
      return;
    }
    const start = node.scrollLeft;
    const startedAt = performance.now();
    const tick = (now: number) => {
      if (!motionEnabled()) { node.scrollLeft = target; scrollFrame = 0; return; }
      const progress = Math.min(1, (now - startedAt) / 160);
      // A short ease-out feels responsive and leaves no browser-wide smooth-scroll state.
      node.scrollLeft = start + (target - start) * (1 - (1 - progress) ** 3);
      if (progress < 1) scrollFrame = requestAnimationFrame(tick);
      else scrollFrame = 0;
    };
    scrollFrame = requestAnimationFrame(tick);
  };
  const revealActive = (smooth = false) => {
    if (node.closest('.compact-tabs') || !node.clientWidth) return;
    const active = node.querySelector<HTMLElement>('.tab-entry.active');
    if (!active) return;
    const strip = node.getBoundingClientRect();
    const tab = active.getBoundingClientRect();
    // Convert visual bounds back to layout pixels for interface zoom. Keep the
    // tab clear of the edge fades, without scrolling any ancestor or page.
    const scale = strip.width / node.clientWidth || 1;
    const left = (tab.left - strip.left) / scale + node.scrollLeft;
    const right = (tab.right - strip.left) / scale + node.scrollLeft;
    const margin = Math.min(20, Math.max(0, (node.clientWidth - (right - left)) / 2));
    if (left - margin < node.scrollLeft) scrollTo(Math.max(0, left - margin), smooth);
    else if (right + margin > node.scrollLeft + node.clientWidth) scrollTo(right + margin - node.clientWidth, smooth);
  };
  const update = () => {
    frame = 0;
    if (revealPending) {
      const smooth = revealPending === 'smooth';
      revealPending = null;
      revealActive(smooth);
    }
    const remaining = node.scrollWidth - node.clientWidth - node.scrollLeft;
    node.style.setProperty('--tab-fade-left', node.scrollLeft > 1 ? '20px' : '0px');
    node.style.setProperty('--tab-fade-right', remaining > 1 ? '20px' : '0px');
  };
  const schedule = () => { if (!frame) frame = requestAnimationFrame(update); };
  const scheduleReveal = (smooth = false) => {
    // Resizes and tab drag reconciliation always win over a pending selection animation.
    revealPending = smooth && revealPending !== 'immediate' ? 'smooth' : 'immediate';
    schedule();
  };
  const resize = new ResizeObserver(() => { cancelScroll(); scheduleReveal(); });
  const observeTabs = () => {
    resize.disconnect();
    resize.observe(node);
    node.querySelectorAll('.tab-entry').forEach(tab => resize.observe(tab));
    scheduleReveal();
  };
  observeTabs();
  const mutations = new MutationObserver(records => {
    if (records.some(record => record.type === 'childList')) observeTabs();
    const active = node.querySelector('.tab-entry.active');
    if (active !== selected) {
      selected = active;
      const smooth = performance.now() - deliberateSelection < 600 && deliberateSelection > 0;
      deliberateSelection = 0;
      scheduleReveal(smooth);
    }
    else schedule();
  });
  const markDeliberateSelection = (event: Event) => {
    cancelScroll();
    if ((event.target as Element | null)?.closest('.tab-entry, .task-select, .channel-row')) deliberateSelection = performance.now();
  };
  const markKeyboardSelection = (event: KeyboardEvent) => {
    if ([' ', 'Enter', 'ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) markDeliberateSelection(event);
    if ((event.metaKey || event.ctrlKey) && (/^[1-9]$/.test(event.key) || ['Tab', '[', ']', 'ArrowLeft', 'ArrowRight'].includes(event.key))) deliberateSelection = performance.now();
  };
  mutations.observe(node, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ['class'] });
  node.addEventListener('scroll', schedule, { passive: true });
  document.addEventListener('pointerdown', markDeliberateSelection, { passive: true, capture: true });
  document.addEventListener('keydown', markKeyboardSelection, true);
  node.addEventListener('wheel', cancelScroll, { passive: true });
  node.addEventListener('touchstart', cancelScroll, { passive: true });
  const fontLoaded = () => scheduleReveal();
  document.fonts?.addEventListener('loadingdone', fontLoaded);
  schedule();
  return { destroy() {
    cancelAnimationFrame(frame);
    cancelScroll();
    resize.disconnect();
    mutations.disconnect();
    node.removeEventListener('scroll', schedule);
    document.removeEventListener('pointerdown', markDeliberateSelection, true);
    document.removeEventListener('keydown', markKeyboardSelection, true);
    node.removeEventListener('wheel', cancelScroll);
    node.removeEventListener('touchstart', cancelScroll);
    document.fonts?.removeEventListener('loadingdone', fontLoaded);
  } };
}
