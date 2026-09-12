import { writable } from 'svelte/store';

// These panels live at the route root so their secure sessions survive
// navigation, sidebar changes, and closing the Settings surface.
export const workspaceShareOpen = writable(false);
export const remoteControlOpen = writable(false);
