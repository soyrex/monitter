import type { ComputerActivity, Message, RunEvent, Task } from './types';

/** Only lifecycle events from the currently owned turn can activate this panel. */
export function activeComputerTools(task: Task | null, messages: Message[], events: RunEvent[]): ComputerActivity[] {
  if (task?.status !== 'running') return [];
  const start = messages.filter(message => message.role === 'user').at(-1)?.createdAt;
  if (start === undefined) return [];
  const active = new Map<string, ComputerActivity>();
  for (const event of events) {
    if (event.createdAt < start) continue;
    if (event.kind === 'status' && ['turn.completed', 'turn.failed'].includes(event.title)) active.clear();
    if (event.kind !== 'computer') continue;
    try {
      const data = JSON.parse(event.detail);
      if (typeof data.id !== 'string') continue;
      if (data.phase === 'completed') active.delete(data.id);
      else if (data.phase === 'started') active.set(data.id, {
        id: data.id, tool: typeof data.tool === 'string' ? data.tool : 'Computer use',
        summary: typeof data.summary === 'string' ? data.summary : '',
      });
    } catch { /* An unrecognised record must never imply live control. */ }
  }
  return [...active.values()];
}
