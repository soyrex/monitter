/** Keep the sidebar wordmark readable when its controls consume the width. */
export function responsiveBrand(node: HTMLElement, _sidebarCompressed = false) {
  let observer: ResizeObserver | undefined;
  let observed: Element[] = [];

  const setCompact = (value: boolean) => {
    const next = value ? 'true' : 'false';
    if (node.dataset.compactWordmark !== next) node.dataset.compactWordmark = next;
  };

  const measure = () => {
    const full = node.querySelector<HTMLElement>('.brand-full');
    const views = node.querySelector<HTMLElement>('.sidebar-views');
    if (!full || !views) { delete node.dataset.compactWordmark; return; }
    const style = getComputedStyle(node);
    const padding = parseFloat(style.paddingLeft) + parseFloat(style.paddingRight);
    const gap = parseFloat(style.columnGap || style.gap) || 0;
    const available = node.clientWidth - padding;
    // offsetWidth/clientWidth are CSS layout units, so this remains correct
    // under Tauri/browser zoom without consulting device pixels.
    setCompact(available < full.offsetWidth + views.offsetWidth + gap + 1);
  };

  const observe = () => {
    observer?.disconnect();
    const full = node.querySelector<HTMLElement>('.brand-full');
    const views = node.querySelector<HTMLElement>('.sidebar-views');
    observed = [node, full, views].filter((item): item is Element => !!item);
    observer = new ResizeObserver(measure);
    observed.forEach(item => observer?.observe(item));
    measure();
  };
  observe();
  return { update: observe, destroy() { observer?.disconnect(); } };
}
