import { floating } from './floating';

export interface TooltipOptions {
  content: string;
  side?: 'below' | 'right' | 'above';
}

/** Reusable accessible tooltip for hover and keyboard-focusable controls. */
export function tooltip(node: HTMLElement, initial: string | TooltipOptions) {
  let options = normalize(initial);
  let popup: HTMLDivElement | undefined;
  let floatingHandle: ReturnType<typeof floating> | undefined;
  // The LAN UI can run from an HTTP IP address, where randomUUID() is not
  // exposed because the page is not a secure context. Tooltips are installed
  // during hydration, so keep their IDs usable on local HTTP as well.
  const id = `monitter-tooltip-${globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random()}`}`;
  const originalDescribedBy = node.getAttribute('aria-describedby');

  function normalize(value: string | TooltipOptions): TooltipOptions {
    return typeof value === 'string' ? { content: value } : value;
  }

  function show() {
    if (popup || !options.content.trim()) return;
    popup = document.createElement('div');
    popup.id = id;
    popup.className = 'monitter-tooltip';
    popup.setAttribute('role', 'tooltip');
    popup.textContent = options.content;
    // Keep the tooltip in the trigger's DOM subtree so it inherits app theme
    // variables while the popover API lifts it above clipping containers.
    node.append(popup);
    node.setAttribute('aria-describedby', [originalDescribedBy, id].filter(Boolean).join(' '));
    floatingHandle = floating(popup, { anchor: node, side: options.side ?? 'above', focus: false });
  }

  function hide() {
    floatingHandle?.destroy();
    floatingHandle = undefined;
    popup?.remove();
    popup = undefined;
    if (originalDescribedBy) node.setAttribute('aria-describedby', originalDescribedBy);
    else node.removeAttribute('aria-describedby');
  }

  function onFocusOut(event: FocusEvent) {
    if (!event.relatedTarget || !node.contains(event.relatedTarget as Node)) hide();
  }

  node.addEventListener('pointerenter', show);
  node.addEventListener('pointerleave', hide);
  node.addEventListener('focusin', show);
  node.addEventListener('focusout', onFocusOut);

  return {
    update(value: string | TooltipOptions) {
      const previousSide = options.side;
      options = normalize(value);
      if (popup) {
        popup.textContent = options.content;
        if (options.side !== previousSide) {
          floatingHandle?.destroy();
          floatingHandle = floating(popup, { anchor: node, side: options.side ?? 'above', focus: false });
        }
      }
    },
    destroy() {
      node.removeEventListener('pointerenter', show);
      node.removeEventListener('pointerleave', hide);
      node.removeEventListener('focusin', show);
      node.removeEventListener('focusout', onFocusOut);
      hide();
    },
  };
}
