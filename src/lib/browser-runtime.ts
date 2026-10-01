import type { BrowserBounds } from '$lib/types';
/** Process-local ownership of live native pages, shared across pane remounts.
 * Only explicit close/unload removes a page; workspace switches merely hide it. */
export const liveBrowserTabs = new Set<string>();

// Every pane operates the same native children; serialize and dedupe globally.
export const browserLayoutRuntime = {
  bounds: {} as Record<string, BrowserBounds>,
  revisions: new Map<string, number>(),
  applied: new Map<string, string>(),
  tail: Promise.resolve() as Promise<void>,
};

// Old pane teardown must not hide a page already mounted in a new pane.
const viewportOwners = new Map<string, symbol>();
export function claimBrowserViewport(id: string) {
  const token = Symbol(id);
  viewportOwners.set(id, token);
  return {
    owns: () => viewportOwners.get(id) === token,
    release: () => { if (viewportOwners.get(id) === token) viewportOwners.delete(id); },
  };
}
