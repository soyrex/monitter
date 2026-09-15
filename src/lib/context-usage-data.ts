import type { Message, RunUsageSummary, Task, UsageOverview } from './types';

export type ComposerContextUsage =
  | { status: 'available'; used: number; size: number; usedPercent: number; model: string | null }
  | { status: 'unavailable'; reason: string };

const contextCleared = (message: Message) =>
  message.role === 'system'
  && message.text === 'Context Cleared'
  && !message.senderAgentId
  && !message.collaborationId
  && !(message.attachments?.length);

function matchingRun(task: Task, runs: RunUsageSummary[], clearedAt: number | null): RunUsageSummary | null {
  const model = task.model.trim() || null;
  return runs
    .filter(run => run.taskId === task.id
      && (run.configuredModel?.trim() || null) === model
      && (clearedAt === null || run.startedAt >= clearedAt)
      && run.context !== null
      && Number.isFinite(run.context.used) && run.context.used >= 0
      && Number.isFinite(run.context.size) && run.context.size > 0)
    .sort((left, right) => right.startedAt - left.startedAt)[0] ?? null;
}

/**
 * Context capacity belongs to a specific native run. Do not substitute token
 * totals or a model catalogue guess when the harness did not report it.
 */
export function composerContextUsage(
  overview: UsageOverview | null,
  task: Task | null,
  messages: Message[],
): ComposerContextUsage {
  if (!task) return { status: 'unavailable', reason: 'Context usage is unavailable.' };

  const clearedAt = messages.filter(contextCleared).at(-1)?.createdAt ?? null;
  const run = matchingRun(task, overview?.recentRuns ?? [], clearedAt);
  if (!run?.context) {
    return {
      status: 'unavailable',
      reason: clearedAt === null
        ? 'Context usage is unavailable until this model reports its context window.'
        : 'Context usage is unavailable after the context was cleared, until this model reports its new context window.',
    };
  }

  const { used, size } = run.context;
  return {
    status: 'available',
    used,
    size,
    usedPercent: Math.max(0, Math.min(100, (used / size) * 100)),
    model: run.configuredModel?.trim() || null,
  };
}
