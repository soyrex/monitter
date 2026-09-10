import { writable } from 'svelte/store';
// Shared across panes so pending feedback follows a tab if it is moved.
export const autonaming = writable<Record<string, boolean>>({});
