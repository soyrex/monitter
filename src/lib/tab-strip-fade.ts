/** Fade only edges with hidden tabs; compact dropdowns must remain unmasked. */
export function tabStripFade(node: HTMLElement) {
  let frame = 0;
  let revealPending = true;
  let selected: Element | null = null;
  const revealActive = () => {
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
    if (left - margin < node.scrollLeft) node.scrollLeft = Math.max(0, left - margin);
    else if (right + margin > node.scrollLeft + node.clientWidth) node.scrollLeft = right + margin - node.clientWidth;
  };
  const update = () => {
    frame = 0;
    if (revealPending) { revealPending = false; revealActive(); }
    const remaining = node.scrollWidth - node.clientWidth - node.scrollLeft;
    node.style.setProperty('--tab-fade-left', node.scrollLeft > 1 ? '20px' : '0px');
    node.style.setProperty('--tab-fade-right', remaining > 1 ? '20px' : '0px');
  };
  const schedule = () => { if (!frame) frame = requestAnimationFrame(update); };
  const scheduleReveal = () => { revealPending = true; schedule(); };
  const resize = new ResizeObserver(scheduleReveal);
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
    if (active !== selected) { selected = active; scheduleReveal(); }
    else schedule();
  });
  mutations.observe(node, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ['class'] });
  node.addEventListener('scroll', schedule, { passive: true });
  document.fonts?.addEventListener('loadingdone', scheduleReveal);
  schedule();
  return { destroy() {
    cancelAnimationFrame(frame);
    resize.disconnect();
    mutations.disconnect();
    node.removeEventListener('scroll', schedule);
    document.fonts?.removeEventListener('loadingdone', scheduleReveal);
  } };
}
