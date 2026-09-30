/* Test-only frontend probe for a separately packaged native fixture. */
(() => {
  'use strict';
  const MAX = 1000, LIMIT = 60_000;
  const names = ['composer-input', 'composer-keydown', 'messages-wheel', 'chat-controls', 'chat-switch-open-to-mount', 'chat-switch-open-to-first-virtualizer-change', 'frame-gaps'];
  const samples = Object.fromEntries(names.map(name => [name, []]));
  const totals = Object.fromEntries(names.map(name => [name, 0])), runMax = Object.fromEntries(names.map(name => [name, 0]));
  let running = false, started = 0, hiddenAt = document.hidden ? performance.now() : 0, hiddenEpoch = 0, lastFrame = 0, observer, generation = 0, stopTimer, renderTimer;
  const panel = document.createElement('aside');
  panel.id = 'monitter-native-ipc-metrics';
  panel.setAttribute('aria-label', 'Native IPC metrics (test only)');
  panel.style.cssText = 'position:fixed;z-index:2147483647;left:8px;bottom:8px;width:218px;max-height:300px;overflow:auto;padding:8px;background:#111;color:#eee;border:1px solid #555;border-radius:8px;font:11px system-ui;box-shadow:0 4px 22px #0008';
  panel.innerHTML = '<strong>Native IPC metrics (test only)</strong><div style="position:sticky;top:0;padding:4px 0;background:#111"><button data-start>Start</button> <button data-stop disabled>Stop</button> <button data-reset>Reset</button><div data-status>Stopped</div></div><div data-results></div>';
  function mount() { if (!panel.isConnected) document.body?.append(panel); }
  if (document.body) mount(); else document.addEventListener('DOMContentLoaded', mount, { once: true });
  const status = panel.querySelector('[data-status]'), results = panel.querySelector('[data-results]');
  const percentile = (values, p) => values[Math.max(0, Math.ceil(values.length * p) - 1)];
  function render() {
    results.replaceChildren(...names.map(name => {
      const values = samples[name].slice().sort((a, b) => a - b), line = document.createElement('p');
      line.style.margin = '6px 0';
      line.textContent = values.length ? `${name}: n=${values.length}/${totals[name]} median=${percentile(values, .5).toFixed(2)}ms p95=${percentile(values, .95).toFixed(2)}ms max=${runMax[name].toFixed(2)}ms` : `${name}: n=0/${totals[name]}`;
      return line;
    }));
  }
  function record(name, value, epoch = hiddenEpoch) {
    if (!running || document.hidden || epoch !== hiddenEpoch || !Number.isFinite(value) || value < 0) return;
    const values = samples[name];
    if (values.length >= MAX) values.shift();
    values.push(value); totals[name]++; runMax[name] = Math.max(runMax[name], value);
    if (!renderTimer) renderTimer = setTimeout(() => { renderTimer = undefined; render(); }, 250);
  }
  function twoFrames(name, start) {
    const epoch = hiddenEpoch, token = generation;
    requestAnimationFrame(() => requestAnimationFrame(() => { if (token === generation) record(name, performance.now() - start, epoch); }));
  }
  function frameProbe() {
    const token = generation;
    if (!running) return;
    requestAnimationFrame(now => {
      if (!running || token !== generation) return;
      if (!document.hidden && lastFrame && hiddenAt === 0) record('frame-gaps', now - lastFrame);
      lastFrame = now;
      frameProbe();
    });
  }
  function onEvent(event) {
    if (!running || document.hidden) return;
    const target = event.target instanceof Element ? event.target : null;
    if (!target) return;
    let name;
    if (event.type === 'input' && target.matches('textarea')) name = 'composer-input';
    else if (event.type === 'keydown' && target.matches('textarea')) name = 'composer-keydown';
    else if (event.type === 'wheel' && target.closest('.messages')) name = 'messages-wheel';
    else if (event.type === 'click' && target.closest('.task-select,.pane-tab,.pane-expand,.splitter')) name = 'chat-controls';
    if (name) twoFrames(name, performance.now());
  }
  function start() {
    if (running) return;
    running = true; started = performance.now(); status.textContent = 'Running (auto-stops after 60s)';
    lastFrame = 0; frameProbe();
    for (const type of ['input', 'keydown', 'wheel', 'click']) document.addEventListener(type, onEvent, { capture: true, passive: type === 'wheel' });
    if ('PerformanceObserver' in window) {
      try {
        const token = generation;
        observer = new PerformanceObserver(list => { if (token !== generation || !running) return; for (const entry of list.getEntries()) {
          const name = entry.name === 'monitter.chat-switch.open-to-mount' ? 'chat-switch-open-to-mount' : entry.name === 'monitter.chat-switch.open-to-first-virtualizer-change' ? 'chat-switch-open-to-first-virtualizer-change' : null;
          if (name) record(name, entry.duration);
        } });
        observer.observe({ type: 'measure' });
      } catch { observer = undefined; }
    }
    stopTimer = setTimeout(() => { if (running) stop(); }, LIMIT);
    panel.querySelector('[data-start]').disabled = true; panel.querySelector('[data-stop]').disabled = false;
  }
  function stop() {
    if (!running) return;
    running = false; generation++; clearTimeout(stopTimer); clearTimeout(renderTimer); stopTimer = renderTimer = undefined; observer?.disconnect(); observer = undefined; status.textContent = `Stopped (${Math.round(performance.now() - started)}ms)`;
    for (const type of ['input', 'keydown', 'wheel', 'click']) document.removeEventListener(type, onEvent, true);
    panel.querySelector('[data-start]').disabled = false; panel.querySelector('[data-stop]').disabled = true; render();
  }
  function reset() { const wasRunning = running; if (wasRunning) stop(); generation++; clearTimeout(stopTimer); clearTimeout(renderTimer); stopTimer = renderTimer = undefined; for (const values of Object.values(samples)) values.length = 0; for (const name of names) { totals[name] = 0; runMax[name] = 0; } render(); if (wasRunning) start(); }
  document.addEventListener('visibilitychange', () => { hiddenEpoch++; lastFrame = 0; if (document.hidden) hiddenAt = performance.now(); else hiddenAt = 0; });
  panel.querySelector('[data-start]').addEventListener('click', start);
  panel.querySelector('[data-stop]').addEventListener('click', stop);
  panel.querySelector('[data-reset]').addEventListener('click', reset);
  const perfUrl = new URL(location.href); perfUrl.searchParams.set('monitter-perf', '1'); history.replaceState(history.state, '', perfUrl);
  render();
})();
