/** Keep the textarea as tall as its contents; let the enclosing composer handle overflow. */
export function autoGrowTextarea(node: HTMLTextAreaElement, _value?: string) {
  let frame = 0;
  let width = node.clientWidth;
  let lastValue = node.value;
  let pendingForceMeasure = false;

  const resize = (forceMeasure = false) => {
    const value = node.value;
    // Growing text can be measured at the current height. Resetting to auto on
    // every keystroke needlessly repaints the translucent composer in WebKit.
    if (forceMeasure || (value !== lastValue && value.length <= lastValue.length)) {
      node.style.height = 'auto';
    }
    lastValue = value;
    const height = `${node.scrollHeight}px`;
    if (node.style.height !== height) node.style.height = height;
  };
  const schedule = (forceMeasure = false) => {
    pendingForceMeasure ||= forceMeasure;
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      const force = pendingForceMeasure;
      pendingForceMeasure = false;
      resize(force);
    });
  };
  const observer = new ResizeObserver(() => {
    const nextWidth = node.clientWidth;
    if (nextWidth !== width) {
      const grew = nextWidth > width;
      width = nextWidth;
      schedule(grew);
    }
  });

  node.addEventListener('input', onInput);
  observer.observe(node);
  schedule(true);

  function onInput() { resize(); }

  return {
    update() { schedule(); },
    destroy() {
      cancelAnimationFrame(frame);
      observer.disconnect();
      node.removeEventListener('input', onInput);
      node.style.height = '';
    },
  };
}
