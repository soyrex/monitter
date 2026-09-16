/** Keep the active-surface indicator on the foremost mounted modal, including
 * its exit animation. Registration avoids observing busy transcript DOM trees. */
const modals: { node: HTMLElement; topLayer: boolean }[] = [];

function updateActiveModal() {
  let active: HTMLElement | undefined;
  let highestLayer = -Infinity;
  for (const { node: modal, topLayer } of modals) {
    // Standard modals use a backdrop layer; standalone dialogs (Sharing, Vim)
    // put their z-index on the dialog itself. Native dialogs/popovers win both.
    const ownLayer = Number.parseInt(getComputedStyle(modal).zIndex, 10) || 0;
    const backdropLayer = modal.parentElement ? Number.parseInt(getComputedStyle(modal.parentElement).zIndex, 10) || 0 : 0;
    const layer = topLayer ? Infinity : Math.max(ownLayer, backdropLayer);
    if (layer >= highestLayer) { active = modal; highestLayer = layer; }
  }
  for (const { node: modal } of modals) modal.toggleAttribute('data-active-modal', modal === active);
}

export function activeModal(node: HTMLElement, topLayer = false) {
  const entry = { node, topLayer };
  modals.push(entry);
  updateActiveModal();
  return {
    destroy() {
      const index = modals.indexOf(entry);
      if (index >= 0) modals.splice(index, 1);
      node.removeAttribute('data-active-modal');
      updateActiveModal();
    },
  };
}
