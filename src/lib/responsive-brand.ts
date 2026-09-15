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
    const actions = node.querySelector<HTMLElement>('.brand-actions');
    if (!full || !actions) { delete node.dataset.compactWordmark; return; }
    const style = getComputedStyle(node);
    const leadingSpace = parseFloat(style.paddingLeft) || 0;
    const trailingSpace = actions.offsetWidth + (parseFloat(style.paddingRight) || 0);
    const available = node.clientWidth - 2 * Math.max(leadingSpace, trailingSpace);
    // offsetWidth/clientWidth are CSS layout units, so this remains correct
    // under Tauri/browser zoom without consulting device pixels.
    setCompact(available < full.offsetWidth + 1);
  };

  const observe = () => {
    observer?.disconnect();
    const full = node.querySelector<HTMLElement>('.brand-full');
    const actions = node.querySelector<HTMLElement>('.brand-actions');
    observed = [node, full, actions].filter((item): item is HTMLElement => !!item);
    observer = new ResizeObserver(measure);
    observed.forEach(item => observer?.observe(item));
    measure();
  };
  observe();
  return { update: observe, destroy() { observer?.disconnect(); } };
}
