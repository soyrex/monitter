<script lang="ts">
  import { Goal as GoalIcon, Monitor, Square } from '@lucide/svelte';
  import type { Goal, ComputerActivity } from '$lib/types';
  let { goal, goalNote = "", tools, onstop, disabled = false }: {
    goal: Goal | null; goalNote?: string; tools: ComputerActivity[]; onstop: () => unknown; disabled?: boolean;
  } = $props();
  const number = (value: number) => new Intl.NumberFormat().format(value);
  const label = (status: string) => ({active:'Active',paused:'Paused',blocked:'Blocked',usageLimited:'Usage limit',budgetLimited:'Budget reached'}[status] ?? status);
</script>

{#if (goal && goal.status !== 'complete') || goalNote || tools.length}
  <div class="task-activity">
    {#if goal && goal.status !== 'complete'}
      <section class="goal-card" aria-label="Task goal">
        <div class="activity-label"><GoalIcon size={14} /><strong>Goal</strong><span class="status">{label(goal.status)}</span></div>
        <details><summary title={goal.objective}>{goal.objective}</summary><p>{goal.objective}</p></details>
        {#if goal.tokensUsed !== undefined}<div class="goal-usage">
          <span>{number(goal.tokensUsed)}{goal.tokenBudget ? ` / ${number(goal.tokenBudget)}` : ''} tokens</span>
          {#if goal.tokenBudget}<progress aria-label="Goal token budget used" value={goal.tokensUsed} max={goal.tokenBudget}></progress>{/if}
        </div>{/if}
      </section>
    {/if}
    {#if goalNote}<section class="goal-card" aria-label="Last goal update">
      <div class="activity-label"><GoalIcon size={14}/><strong>Last goal update</strong></div>
      <details><summary title={goalNote}>{goalNote}</summary><p>{goalNote}</p></details>
    </section>{/if}
    {#if tools.length}
      <section class="computer-card" aria-label="Computer use activity">
        <div class="computer-info"><div class="activity-label"><Monitor size={14}/><strong>Using the computer</strong></div>
          {#each tools as tool (tool.id)}<p title={tool.summary}>{tool.summary || tool.tool}</p>{/each}
        </div>
        <button aria-label="Stop computer use" {disabled} onclick={onstop}><Square size={12}/>Stop</button>
      </section>
    {/if}
  </div>
{/if}

<style>
  .task-activity { flex: none; min-height: 0; max-height: 25%; overflow: auto; overscroll-behavior: contain; margin: 10px 20px 0; font-size: 12px; }
  section { border: 1px solid var(--line); background: var(--panel); border-radius: 8px; padding: 10px 12px; }
  section + section { margin-top: 8px; }
  .activity-label { display: flex; align-items: center; gap: 7px; color: var(--accent-ink); }
  strong { font-weight: 600; }
  .status { margin-left: auto; font: 10px var(--mono); color: var(--muted); }
  details { margin-top: 7px; }
  summary { cursor: pointer; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  p { line-height: 1.5; overflow-wrap: anywhere; }
  .goal-usage { display: flex; align-items: center; gap: 12px; margin-top: 7px; color: var(--muted); font: 10px var(--mono); }
  progress { width: 90px; height: 5px; accent-color: var(--accent); }
  .computer-card { display: flex; align-items: center; justify-content: space-between; gap: 10px; }
  .computer-info { min-width: 0; }
  .computer-info p { margin: 5px 0 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; color: var(--muted); }
  button { display: flex; align-items: center; gap: 5px; flex: none; padding: 6px 8px; border: 1px solid var(--line); border-radius: 5px; }
  button:hover { background: var(--soft); }
  @media (max-height: 500px) { .task-activity { margin: 5px 10px 0; max-height: 18%; } section { padding: 7px 9px; } }
</style>
