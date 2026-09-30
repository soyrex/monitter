// Retain unsent text when Companion Mode closes without adding a new durable
// transcript store. A page reload clears these drafts, just like other local UI state.
const drafts = new Map<string, string>();

export function readCompanionDrafts(): Record<string, string> {
  return Object.fromEntries(drafts);
}

export function saveCompanionDraft(taskId: string, text: string): void {
  if (text) drafts.set(taskId, text);
  else drafts.delete(taskId);
}
