/** Keep the textarea as tall as its contents; let the enclosing composer handle overflow. */
export function autoGrowTextarea(node: HTMLTextAreaElement, _value?: string) {
  let frame = 0;
  let width = node.clientWidth;

  const resize = () => {
    node.style.height = 'auto';
    node.style.height = `${node.scrollHeight}px`;
  };
  const schedule = () => {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(resize);
  };
  const observer = new ResizeObserver(() => {
    const nextWidth = node.clientWidth;
    if (nextWidth !== width) {
      width = nextWidth;
      schedule();
    }
  });

  node.addEventListener('input', resize);
  observer.observe(node);
  schedule();

  return {
    update: schedule,
    destroy() {
      cancelAnimationFrame(frame);
      observer.disconnect();
      node.removeEventListener('input', resize);
      node.style.height = '';
    },
  };
}
