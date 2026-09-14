/** Safari's layout viewport does not shrink with the software keyboard. */
export function mobileViewport(node: HTMLElement) {
  const viewport = window.visualViewport;
  let frame = 0;
  const update = () => {
    frame = 0;
    const height = viewport?.height ?? window.innerHeight;
    const top = viewport?.offsetTop ?? 0;
    const scale = Number.parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--browser-interface-scale')) || 1;
    node.style.setProperty('--mobile-viewport-height', `${height}px`);
    node.style.setProperty('--mobile-viewport-top', `${top}px`);
    node.style.setProperty('--scaled-mobile-viewport-height', `${height / scale}px`);
    node.style.setProperty('--scaled-mobile-viewport-top', `${top / scale}px`);
    node.dataset.keyboardComposer = String(height < 500 && !!document.activeElement?.closest('.composer'));
  };
  const schedule = () => { if (!frame) frame = requestAnimationFrame(update); };
  update();
  viewport?.addEventListener('resize', schedule);
  viewport?.addEventListener('scroll', schedule);
  window.addEventListener('resize', schedule);
  window.addEventListener('monitter:interface-scale', schedule);
  node.addEventListener('focusin', schedule);
  node.addEventListener('focusout', schedule);
  return { destroy() {
    cancelAnimationFrame(frame);
    viewport?.removeEventListener('resize', schedule);
    viewport?.removeEventListener('scroll', schedule);
    window.removeEventListener('resize', schedule);
    window.removeEventListener('monitter:interface-scale', schedule);
    node.removeEventListener('focusin', schedule);
    node.removeEventListener('focusout', schedule);
  } };
}
