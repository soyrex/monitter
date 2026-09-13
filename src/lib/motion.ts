import type { Action } from 'svelte/action';
import { get, writable } from 'svelte/store';

export type MotionPreference = 'system' | 'subtle' | 'off';

const storageKey = 'monitter.appearance.motion.v1';
const hasWindow = typeof window !== 'undefined';
const reducedMotion = hasWindow && typeof window.matchMedia === 'function'
  ? window.matchMedia('(prefers-reduced-motion: reduce)')
  : null;

function normalize(value: unknown): MotionPreference {
  return value === 'subtle' || value === 'off' || value === 'system' ? value : 'system';
}

function loadPreference(): MotionPreference {
  if (!hasWindow) return 'system';
  try { return normalize(window.localStorage.getItem(storageKey)); }
  catch { return 'system'; }
}

/** Client-local motion setting. The operating-system reduced-motion preference remains a hard limit. */
export const motionPreference = writable<MotionPreference>(loadPreference());
const motionAllowed = writable(false);
const activeAnimations = new Set<Animation>();

function updatePolicy(): void {
  const enabled = get(motionPreference) !== 'off' && !reducedMotion?.matches;
  motionAllowed.set(enabled);
  if (!enabled) {
    for (const animation of activeAnimations) animation.cancel();
    activeAnimations.clear();
  }
}

motionPreference.subscribe(() => updatePolicy());
if (reducedMotion) reducedMotion.addEventListener('change', updatePolicy);

/** Returns the effective policy now; safe to call during SSR. */
export function motionEnabled(): boolean {
  return get(motionPreference) !== 'off' && !reducedMotion?.matches;
}

export function setMotionPreference(value: MotionPreference): void {
  const next = normalize(value);
  motionPreference.set(next);
  if (!hasWindow) return;
  try { window.localStorage.setItem(storageKey, next); } catch { /* Live preference still applies. */ }
}

function applyDocumentPolicy(): void {
  if (typeof document !== 'undefined') {
    document.documentElement.dataset.motion = motionEnabled() ? 'subtle' : 'off';
  }
}

/** Apply the effective policy to the app root. Use once on AppSurface's retained root. */
export const initMotion: Action<HTMLElement> = () => {
  applyDocumentPolicy();
  const unsubscribe = motionAllowed.subscribe(applyDocumentPolicy);
  // A visually exiting dialog is inert, but its backdrop still owns interaction.
  // Prevent shortcuts from reaching the newly exposed workspace during that gap.
  const guardClosing = (event: KeyboardEvent) => {
    if (!document.querySelector('[data-motion-closing="true"]')) return;
    if (document.querySelector('[aria-modal="true"]:not([data-motion-closing="true"])')) return;
    event.preventDefault();
    event.stopImmediatePropagation();
  };
  window.addEventListener('keydown', guardClosing, true);
  return { destroy() { unsubscribe(); window.removeEventListener('keydown', guardClosing, true); } };
};

const nodeAnimations = new WeakMap<Element, Animation>();

/**
 * Starts one interruptible decorative animation for a node. Finished animations
 * are cancelled so transient Web Animations styles never become retained state.
 */
export function animateMotion(
  node: Element,
  keyframes: Keyframe[] | PropertyIndexedKeyframes,
  options: KeyframeAnimationOptions,
): Animation | null {
  nodeAnimations.get(node)?.cancel();
  if (!motionEnabled() || typeof node.animate !== 'function') return null;

  const animation = node.animate(keyframes, { fill: 'both', ...options });
  nodeAnimations.set(node, animation);
  activeAnimations.add(animation);
  const clean = () => {
    activeAnimations.delete(animation);
    if (nodeAnimations.get(node) === animation) {
      nodeAnimations.delete(node);
      animation.cancel();
    }
  };
  void animation.finished.then(clean, () => activeAnimations.delete(animation));
  return animation;
}

export type MotionViewOptions = {
  key: string | number;
  x?: number;
  y?: number;
  duration?: number;
  opacity?: number;
  initial?: boolean;
  enabled?: boolean;
};

/** A retained-view entrance: animate after DOM updates, without remounting the view. */
export const motionView: Action<HTMLElement, MotionViewOptions> = (node, initialOptions) => {
  let options = initialOptions;
  let previousKey = options.key;
  let frame = 0;

  const cancel = () => { cancelAnimationFrame(frame); frame = 0; nodeAnimations.get(node)?.cancel(); };
  const enter = () => {
    cancel();
    if (options.enabled === false || !motionEnabled() || !node.getClientRects().length || node.closest('[inert]')) return;
    const x = options.x ?? 0;
    const y = options.y ?? 4;
    const clipped = node.classList.contains('task-tree');
    animateMotion(node, [
      { opacity: options.opacity ?? 0.88, transform: `translate(${x}px, ${y}px)`, ...(clipped ? { clipPath: 'inset(0 0 20% 0)' } : {}) },
      { opacity: 1, transform: 'translate(0, 0)', ...(clipped ? { clipPath: 'inset(0)' } : {}) },
    ], { duration: options.duration ?? 150, easing: 'ease-out' });
  };
  const unsubscribe = motionAllowed.subscribe((enabled) => { if (!enabled) cancel(); });
  const schedule = () => {
    cancel();
    // Svelte may update an action before removing the view's hidden class.
    frame = requestAnimationFrame(() => { frame = 0; enter(); });
  };

  if (options.initial) schedule();
  return {
    update(next) {
      options = next;
      if (next.enabled === false) cancel();
      else if (previousKey !== next.key) schedule();
      previousKey = next.key;
    },
    destroy() { unsubscribe(); cancel(); },
  };
};
