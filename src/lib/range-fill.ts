/** Keep the painted track in sync with dragging, keyboard input and resets. */
export function rangeFill(node: HTMLInputElement, value: number) {
  function paint(current: number) {
    const min = Number(node.min || 0);
    const max = Number(node.max || 100);
    const fraction = max > min ? Math.max(0, Math.min(1, (current - min) / (max - min))) : 0;
    node.style.setProperty('--range-fill', `${fraction * 100}%`);
  }
  const input = () => paint(node.valueAsNumber);
  paint(value);
  node.addEventListener('input', input);
  return {
    update: paint,
    destroy() { node.removeEventListener('input', input); },
  };
}
