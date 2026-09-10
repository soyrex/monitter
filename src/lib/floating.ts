/** Put transient menus in the browser's top layer, outside pane overflow. */
export function floating(
  node: HTMLElement,
  options: { anchor: HTMLElement; side?: 'below' | 'right' | 'above'; focus?: boolean },
) {
  const anchor = options.anchor;
  node.setAttribute('popover', 'manual');
  node.showPopover();

  function position() {
    const rect = anchor.getBoundingClientRect();
    const margin = 8;
    node.style.maxWidth = `${Math.max(0, window.innerWidth - margin * 2)}px`;
    node.style.maxHeight = `${Math.max(0, window.innerHeight - margin * 2)}px`;
    const width = node.offsetWidth, height = node.offsetHeight;
    const preferredLeft = options.side === 'right' ? rect.right + margin : rect.right - width;
    const preferredTop = options.side === 'right' ? rect.top : options.side === 'above' ? rect.top - height - 6 : rect.bottom + 6;
    const left = Math.max(margin, Math.min(preferredLeft, window.innerWidth - width - margin));
    const top = Math.max(margin, Math.min(preferredTop, window.innerHeight - height - margin));
    node.style.left = `${left}px`;
    node.style.top = `${top}px`;
  }

  position();
  const observer = new ResizeObserver(position);
  observer.observe(node);
  window.addEventListener('resize', position);
  window.addEventListener('scroll', position, true);
  if (options.focus !== false) node.querySelector<HTMLElement>('button:not(:disabled)')?.focus();
  return {
    destroy() {
      observer.disconnect();
      window.removeEventListener('resize', position);
      window.removeEventListener('scroll', position, true);
      if (node.matches(':popover-open')) node.hidePopover();
      if (node.contains(document.activeElement)) anchor.focus();
    },
  };
}
