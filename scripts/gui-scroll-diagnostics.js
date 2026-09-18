// Loaded only by the explicitly synthetic loopback fixture, never the packaged app.
(() => {
  let sequence = 0;
  let pending = 0;
  const round = value => Math.round(value);
  function inspect(reason) {
    const panel = document.querySelector('#synthetic-scroll-diagnostics');
    if (!panel) return;
    const lines = [...document.querySelectorAll('.messages')].map((viewport, pane) => {
      const rect = viewport.getBoundingClientRect();
      const rows = [...viewport.querySelectorAll('.transcript-row')].map(row => {
        const box = row.getBoundingClientRect();
        return { index: Number(row.dataset.index), top: round(box.top - rect.top), height: round(box.height), visible: box.bottom > rect.top && box.top < rect.bottom };
      });
      const visible = rows.filter(row => row.visible);
      const list = viewport.querySelector('.transcript-virtual-list');
      const spacers = list ? [...list.children].filter(child => child.getAttribute('aria-hidden') === 'true') : [];
      const margin = list ? round(list.getBoundingClientRect().top - rect.top + viewport.scrollTop) : null;
      return `pane ${pane + 1}: top=${round(viewport.scrollTop)} client=${viewport.clientHeight} total=${viewport.scrollHeight} margin=${margin}; mounted=${rows.length} indexes=${rows.map(row => row.index).join(',')}; intersect=${visible.length} ${visible.map(row => `${row.index}@${row.top}+${row.height}`).join(',')}; spacer first/last=${round(spacers[0]?.getBoundingClientRect().height ?? 0)}/${round(spacers.at(-1)?.getBoundingClientRect().height ?? 0)}`;
    });
    panel.replaceChildren(...[`${++sequence} ${reason}; mask=${document.documentElement.classList.contains('fixture-mask-off') ? 'off' : 'normal'}`, ...lines].map(line => {
      const paragraph = document.createElement('p');
      paragraph.textContent = line;
      return paragraph;
    }));
  }
  addEventListener('DOMContentLoaded', () => {
    const container = document.querySelector('#synthetic-performance-fixture');
    if (!container) return;
    const style = document.createElement('style');
    style.textContent = '.fixture-mask-off .messages{mask-image:none!important;-webkit-mask-image:none!important}';
    document.head.append(style);
    const inspectButton = document.createElement('button');
    inspectButton.textContent = 'Inspect scroll geometry';
    inspectButton.onclick = () => inspect('manual');
    const maskButton = document.createElement('button');
    maskButton.textContent = 'Toggle fixture scroll mask';
    maskButton.onclick = () => { document.documentElement.classList.toggle('fixture-mask-off'); inspect('mask toggle'); };
    const output = document.createElement('div');
    output.id = 'synthetic-scroll-diagnostics';
    output.setAttribute('aria-label', 'Synthetic scroll geometry');
    output.style.cssText = 'max-height:100px;overflow:auto';
    container.append(inspectButton, maskButton, output);
    inspect('initial');
  });
  addEventListener('wheel', event => {
    if (!(event.target instanceof Element) || !event.target.closest('.messages')) return;
    const request = ++pending;
    requestAnimationFrame(() => requestAnimationFrame(() => { if (request === pending) inspect('wheel + two frames'); }));
    setTimeout(() => { if (request === pending) inspect('wheel + 500ms'); }, 500);
  }, { capture: true, passive: true });
})();
