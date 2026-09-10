(() => {
  'use strict';
  const DEFAULT = '#3f9d6a';
  const KEY = 'agent-harness-design.accent';
  const root = document.documentElement;
  const picker = document.getElementById('accent-colour');
  const output = document.getElementById('accent-value');
  const swatches = [...document.querySelectorAll('[data-accent]')];
  const rgb = hex => [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16));
  const luminance = channels => channels.map(c => {
    const v = c / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  }).reduce((sum, v, i) => sum + v * [0.2126, 0.7152, 0.0722][i], 0);
  const contrast = (a, b) => {
    const x = luminance(a), y = luminance(b);
    return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
  };
  const readable = (colour, background, target) => {
    for (let step = 0; step <= 100; step++) {
      const adjusted = colour.map(c => Math.round(c + (target - c) * step / 100));
      if (contrast(adjusted, background) >= 4.5) return adjusted;
    }
    return [target, target, target];
  };
  function apply(value, persist = true) {
    if (!/^#[0-9a-f]{6}$/i.test(value)) return;
    const hex = value.toLowerCase();
    const colour = rgb(hex);
    const lightInk = readable(colour, rgb('#f2ede3'), 0);
    const darkInk = readable(colour, rgb('#1f1f23'), 255);
    const white = [255, 255, 255], black = [0, 0, 0];
    root.style.setProperty('--accent', hex);
    root.style.setProperty('--accent-rgb', colour.join(', '));
    root.style.setProperty('--accent-ink', `rgb(${lightInk.join(', ')})`);
    root.style.setProperty('--accent-dark-ink', `rgb(${darkInk.join(', ')})`);
    root.style.setProperty('--accent-dark-rgb', darkInk.join(', '));
    root.style.setProperty('--on-accent', contrast(colour, white) >= contrast(colour, black) ? '#ffffff' : '#000000');
    picker.value = hex;
    output.value = hex.toUpperCase();
    swatches.forEach(button => button.setAttribute('aria-pressed', String(button.dataset.accent === hex)));
    if (persist) {
      try { localStorage.setItem(KEY, hex); } catch { /* The picker also works when storage is unavailable. */ }
    }
  }
  picker.addEventListener('input', () => apply(picker.value));
  swatches.forEach(button => button.addEventListener('click', () => apply(button.dataset.accent)));
  document.getElementById('accent-reset').addEventListener('click', () => apply(DEFAULT));
  let saved;
  try { saved = localStorage.getItem(KEY); } catch { /* Use the default. */ }
  apply(/^#[0-9a-f]{6}$/i.test(saved || '') ? saved : DEFAULT, false);
})();
