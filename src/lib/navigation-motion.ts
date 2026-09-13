import { animateMotion, motionEnabled } from './motion';

const exits = new WeakMap<HTMLElement, () => void>();
/** A short-lived, inert visual copy; never retains live components or focus. */
export function outgoingVisual(node: HTMLElement | null | undefined, x = 0, y = 0, duration = 100) {
  if (!node?.parentElement || !motionEnabled() || !node.getClientRects().length || node.closest('[inert]')) return;
  const parent = node.parentElement;
  exits.get(parent)?.();
  // Large sidebars should not pay for a second DOM tree just to fade it out.
  // Keep their incoming entrance, but bound the optional outgoing snapshot work.
  const walker = document.createTreeWalker(node, NodeFilter.SHOW_ELEMENT);
  for (let count = 0; walker.nextNode(); count++) {
    if (count >= 512) return;
  }
  const ghost = node.cloneNode(true) as HTMLElement;
  ghost.inert = true;
  ghost.setAttribute('aria-hidden', 'true');
  ghost.dataset.motionGhost = 'true';
  for (const element of [ghost, ...ghost.querySelectorAll('*')]) {
    for (const name of ['id', 'aria-live', 'role', 'autofocus']) element.removeAttribute(name);
  }
  // Consumers use positioned parents, keeping clipping, theme and scale intact.
  Object.assign(ghost.style, {
    position: 'absolute', top: `${node.offsetTop}px`, left: `${node.offsetLeft}px`,
    width: `${node.offsetWidth}px`, height: `${node.offsetHeight}px`, margin: '0',
    pointerEvents: 'none', overflow: 'hidden', zIndex: '20',
  });
  parent.append(ghost);
  ghost.scrollTop = node.scrollTop;
  const animation = animateMotion(ghost, [
    { opacity: 1, transform: 'translate(0, 0)' },
    { opacity: 0, transform: `translate(${x}px, ${y}px)` },
  ], { duration, easing: 'ease-in' });
  const clean = () => { ghost.remove(); if (exits.get(parent) === clean) exits.delete(parent); };
  exits.set(parent, clean);
  if (animation) void animation.finished.then(clean, clean); else clean();
  return clean;
}

/** Animate only the conversation's display, never its composer or a terminal. */
export function conversationMotion(node: HTMLElement, options: { key: string; active: boolean }) {
  let previous = options.key;
  let frame = 0;
  let animation: Animation | null = null;
  return {
    update(next: typeof options) {
      if (next.key === previous) return;
      previous = next.key;
      cancelAnimationFrame(frame);
      animation?.cancel();
      if (!next.active) return;
      frame = requestAnimationFrame(() => {
        const content = node.querySelector<HTMLElement>('.conversation .message-pane');
        if (!content || !content.getClientRects().length || content.closest('[inert]')) return;
        animation = animateMotion(content, [{ opacity: 0.85 }, { opacity: 1 }], { duration: 120, easing: 'ease-out' });
      });
    },
    destroy() { cancelAnimationFrame(frame); animation?.cancel(); },
  };
}

/** Only local pending sends or a live reply replacing thinking get an entrance. */
export function messageArrival(node: HTMLElement) {
  let frame = 0;
  const animations = new Set<Animation>();
  const observer = new MutationObserver(records => {
    const added = records.flatMap(record => [...record.addedNodes]).filter((item): item is HTMLElement => item instanceof HTMLElement && item.matches('.message'));
    const replacedThinking = records.some(record => [...record.removedNodes].some(item => item instanceof HTMLElement && item.matches('.reasoning-pending')));
    if (added.length !== 1 || !motionEnabled() || document.visibilityState !== 'visible') return;
    const message = added[0];
    if (!message.matches('.optimistic-message') && !(replacedThinking && message.dataset.liveEntry === 'true')) return;
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      const viewport = node.closest('.messages');
      if (!viewport || node.closest('[inert]') || node.closest('.pane-leaf.dimmed') || !node.getClientRects().length) return;
      const bounds = message.getBoundingClientRect(), clip = viewport.getBoundingClientRect();
      if (bounds.bottom <= clip.top || bounds.top >= clip.bottom) return;
      const animation = animateMotion(message, [{ opacity: 0.6, transform: 'translateY(4px)' }, { opacity: 1, transform: 'translateY(0)' }], { duration: 120, easing: 'ease-out' });
      if (animation) { animations.add(animation); void animation.finished.then(() => animations.delete(animation), () => animations.delete(animation)); }
    });
  });
  observer.observe(node, { childList: true });
  return { destroy() { observer.disconnect(); cancelAnimationFrame(frame); for (const animation of animations) animation.cancel(); } };
}
