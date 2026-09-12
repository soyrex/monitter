/** Safari's layout viewport does not shrink with the software keyboard. */
export function mobileViewport(node: HTMLElement) {
  const viewport = window.visualViewport;
  let frame = 0;
  const update = () => {
    frame = 0;
    const height = viewport?.height ?? window.innerHeight;
    node.style.setProperty('--mobile-viewport-height', `${height}px`);
    node.style.setProperty('--mobile-viewport-top', `${viewport?.offsetTop ?? 0}px`);
    node.dataset.keyboardComposer = String(height < 500 && !!document.activeElement?.closest('.composer'));
  };
  const schedule = () => { if (!frame) frame = requestAnimationFrame(update); };
  update();
  viewport?.addEventListener('resize', schedule);
  viewport?.addEventListener('scroll', schedule);
  window.addEventListener('resize', schedule);
  node.addEventListener('focusin', schedule);
  node.addEventListener('focusout', schedule);
  return { destroy() {
    cancelAnimationFrame(frame);
    viewport?.removeEventListener('resize', schedule);
    viewport?.removeEventListener('scroll', schedule);
    window.removeEventListener('resize', schedule);
    node.removeEventListener('focusin', schedule);
    node.removeEventListener('focusout', schedule);
  } };
}
