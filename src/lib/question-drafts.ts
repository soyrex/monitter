export interface QuestionDraft { values: Record<string, string>; selections: Record<string, string[]> }
// Deliberately process-memory only: answers, including secrets, never enter browser storage.
const drafts = new Map<string, QuestionDraft>();
const resolved = new Set<string>();
export const questionDraftKey = (taskId: string, requestId: string) => JSON.stringify([taskId, requestId]);
const copy = (draft: QuestionDraft): QuestionDraft => ({ values: Object.assign(Object.create(null), draft.values), selections: Object.fromEntries(Object.entries(draft.selections).map(([key, values]) => [key, [...values]])) });
export function readQuestionDraft(key?: string): QuestionDraft { return key && drafts.has(key) ? copy(drafts.get(key)!) : { values: Object.create(null), selections: Object.create(null) }; }
export function saveQuestionDraft(key: string | undefined, draft: QuestionDraft): void { if (key && !resolved.has(key)) drafts.set(key, copy(draft)); }
export function pruneQuestionDrafts(taskId: string, pendingIds: string[]): void {
  const pending = new Set(pendingIds);
  for (const key of drafts.keys()) {
    const [scope, id] = JSON.parse(key);
    if (scope === taskId && !pending.has(id)) { drafts.delete(key); resolved.add(key); }
  }
}
export function clearQuestionDraft(taskId: string, requestId: string): void { const key = questionDraftKey(taskId, requestId); drafts.delete(key); resolved.add(key); }

/** Reconcile against an authoritative complete approval snapshot, including other tasks. */
export function reconcileQuestionDrafts(requests: { taskId: string; id: string; status: string }[]): void {
  const pending = new Set(requests.filter(request => request.status === 'pending').map(request => questionDraftKey(request.taskId, request.id)));
  for (const key of drafts.keys()) if (!pending.has(key)) { drafts.delete(key); resolved.add(key); }
}
