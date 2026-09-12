import { writable } from 'svelte/store';

// These panels live at the route root so their secure sessions survive
// navigation, sidebar changes, and closing the Settings surface.
export const workspaceShareOpen = writable(false);
export const remoteControlOpen = writable(false);

// One root-owned listener, presented in a visible Settings pane. Prefer the
// focused pane and retain other targets so closing it restores the previous one.
export const remoteControlTarget = writable<HTMLElement | null>(null);
const remoteControlTargets = new Map<symbol, { target: HTMLElement; active: boolean }>();

function selectRemoteControlTarget() {
  const visible = [...remoteControlTargets.values()].filter(entry => entry.target.isConnected);
  remoteControlTarget.set((visible.filter(entry => entry.active).at(-1) ?? visible.at(-1))?.target ?? null);
}

export function setRemoteControlTarget(target: HTMLElement, active = true) {
  const token = Symbol('remote-settings');
  remoteControlTargets.set(token, { target, active });
  selectRemoteControlTarget();
  return () => {
    remoteControlTargets.delete(token);
    selectRemoteControlTarget();
  };
}
