import type { BuildRequest, BuildRun, Goal, Message, RunEvent, Snapshot, Task, WorkPlan } from '$lib/types';
import { activeUnifiedSubagent, unifiedSubagentsFromSnapshot, type UnifiedSubagent } from '$lib/unified-subagents';

export interface WatchBuild {
  request: BuildRequest | null;
  run: BuildRun | null;
  title: string;
  label: 'Building' | 'Build needs attention' | 'Waiting for build claim' | 'Preparing build';
  stale: boolean;
}
export interface WatchProjection {
  request: Message | null;
  finalReply: Message | null;
  activePlan: WorkPlan | null;
  finishedPlan: WorkPlan | null;
  progress: RunEvent | null;
  tool: RunEvent | null;
  activity: RunEvent[];
  team: UnifiedSubagent[];
  activeTeam: UnifiedSubagent[];
  finishedTeam: UnifiedSubagent[];
  build: WatchBuild | null;
}
const newest = <T extends { createdAt: number; id: string }>(a: T, b: T) => b.createdAt - a.createdAt || b.id.localeCompare(a.id);
const planOrder = (a: WorkPlan, b: WorkPlan) => b.updatedAt - a.updatedAt || newest(a, b);

/** Usage is accounting, never a percentage of the objective completed. */
export function watchGoal(goal: Goal, plan: WorkPlan | null, progress: RunEvent | null) {
  const labels: Record<string, string> = { active: 'Active goal', paused: 'Goal paused', blocked: 'Goal blocked', complete: 'Goal complete', usageLimited: 'Goal usage limit', budgetLimited: 'Goal budget reached' };
  const valid = (value: number | undefined | null): value is number => typeof value === 'number' && Number.isFinite(value) && value >= 0;
  const usage: string[] = [];
  if (valid(goal.tokensUsed)) usage.push(`${goal.tokensUsed.toLocaleString()}${valid(goal.tokenBudget) && goal.tokenBudget > 0 ? ` / ${goal.tokenBudget.toLocaleString()}` : ''} tokens used`);
  if (valid(goal.timeUsedSeconds)) {
    const seconds = Math.floor(goal.timeUsedSeconds);
    usage.push(`${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')} elapsed`);
  }
  return { label: labels[goal.status] ?? 'Goal status unknown', usage: usage.join(' · '),
    completed: plan?.items.filter(item => item.status === 'completed').length ?? 0,
    total: plan?.items.length ?? 0,
    current: plan?.items.find(item => item.status === 'in_progress')?.title || plan?.items.find(item => item.status === 'blocked')?.title || progress?.title || '',
  };
}

/** Only queue ownership and included requests can attribute a build to this task. */
export function selectWatchBuild(snapshot: Pick<Snapshot, 'buildRequests' | 'buildRuns'>, task: Pick<Task, 'id' | 'projectId'>, now: number): WatchBuild | null {
  if (!task.projectId) return null;
  const requests = (snapshot.buildRequests ?? []).filter(row => row.taskId === task.id && row.projectId === task.projectId);
  const run = (snapshot.buildRuns ?? []).filter(row => row.status === 'running' && row.projectId === task.projectId &&
    (row.ownerTaskId === task.id || requests.some(request => row.requestIds.includes(request.id)))).sort(newest)[0];
  if (run) {
    const request = requests.filter(row => run.requestIds.includes(row.id)).sort(newest)[0] ?? null;
    const stale = run.leaseExpiresAt < now;
    return { run, request, title: request?.title.trim() || 'Current build', stale, label: stale ? 'Build needs attention' : 'Building' };
  }
  const request = requests.filter(row => row.status === 'planned' || row.status === 'ready').sort((a, b) => Number(b.status === 'ready') - Number(a.status === 'ready') || newest(a, b))[0];
  return request ? { request, run: null, title: request.title.trim() || 'Current build', stale: false, label: request.status === 'ready' ? 'Waiting for build claim' : 'Preparing build' } : null;
}

export function selectWatchTeam(team: UnifiedSubagent[], since: number): UnifiedSubagent[] {
  return team.filter(row => activeUnifiedSubagent(row) || Math.max(row.updatedAt, row.launchedAt) >= since)
    .sort((a, b) => Number(activeUnifiedSubagent(b)) - Number(activeUnifiedSubagent(a)) || a.launchedAt - b.launchedAt || a.id.localeCompare(b.id));
}

/** Match desktop Watch: only a settled root answer from the latest turn. */
export function selectWatchFinalReply(messages: Message[], task: Task): Message | null {
  if (!['idle', 'completed', 'succeeded'].includes(task.status)) return null;
  const scoped = messages.filter(row => row.taskId === task.id);
  const latestUser = scoped.filter(row => row.role === 'user').sort(newest)[0];
  if (!latestUser) return null;
  const answer = scoped.filter(row => row.role === 'assistant' && !row.senderAgentId && !row.collaborationId &&
    (!row.phase || row.phase === 'final_answer') && row.createdAt >= latestUser.createdAt).sort(newest)[0];
  return answer && answer.streamStatus !== 'streaming' && answer.text.trim() ? answer : null;
}

/** Presentation only: source records and all mutations remain backend-owned. */
export function projectWatch(snapshot: Snapshot, task: Task, now = Date.now()): WatchProjection {
  const messages = snapshot.messages.filter(row => row.taskId === task.id && row.role === 'user');
  const request = messages.filter(row => !row.senderAgentId && row.text.trim()).sort(newest)[0] ?? null;
  const latestUser = [...messages].sort(newest)[0];
  const plans = (snapshot.workPlans ?? []).filter(row => row.taskId === task.id).sort(planOrder);
  const activePlan = plans.find(row => row.status === 'active') ?? null;
  const finishedPlan = plans.find(row => row.status !== 'active') ?? null;
  const plan = activePlan ?? finishedPlan;
  const events = latestUser ? snapshot.events.filter(row => row.taskId === task.id && row.createdAt >= latestUser.createdAt).sort(newest) : [];
  const team = selectWatchTeam(unifiedSubagentsFromSnapshot(snapshot, task.id), plan?.createdAt ?? latestUser?.createdAt ?? 0);
  return {
    request, finalReply: selectWatchFinalReply(snapshot.messages, task), activePlan, finishedPlan,
    progress: events.find(row => row.kind === 'agent_progress' && row.title.trim()) ?? null,
    tool: events.find(row => (row.kind === 'tool' || row.kind === 'computer') && row.title.trim()) ?? null,
    activity: events.slice(0, 8), team,
    activeTeam: team.filter(activeUnifiedSubagent), finishedTeam: team.filter(row => !activeUnifiedSubagent(row)),
    build: selectWatchBuild(snapshot, task, now),
  };
}
