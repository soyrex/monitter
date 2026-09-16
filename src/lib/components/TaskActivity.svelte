<script lang="ts">
  import { Goal as GoalIcon, Monitor, Square, Trash2 } from '@lucide/svelte';
  import type { Goal, ComputerActivity } from '$lib/types';
  let { goal, goalNote = "", tools, onstop, onclear, clearing = false, clearError = "", disabled = false }: {
    goal: Goal | null; goalNote?: string; tools: ComputerActivity[]; onstop: () => unknown; onclear: () => unknown; clearing?: boolean; clearError?: string; disabled?: boolean;
  } = $props();
  const number = (value: number) => new Intl.NumberFormat().format(value);
  const label = (status: string) => ({active:'Active',paused:'Paused',blocked:'Blocked',usageLimited:'Usage limit',budgetLimited:'Budget reached'}[status] ?? status);
  const sparkles = [
    { x: 3, y: 12, delay: 0 }, { x: 27, y: 5, delay: -.8 },
    { x: 71, y: 7, delay: -1.6 }, { x: 97, y: 48, delay: -.4 },
    { x: 82, y: 94, delay: -1.2 }, { x: 16, y: 94, delay: -2 },
  ];
</script>

{#if (goal && goal.status !== 'complete') || goalNote || tools.length}
  <div class="task-activity">
    {#if goal && goal.status !== 'complete'}
      <section class="goal-card clearable" class:in-progress={goal.status === 'active'} aria-label="Task goal">
        {#if goal.status === 'active'}<div class="goal-sparkles" aria-hidden="true">
          {#each sparkles as sparkle}<span style:left={`${sparkle.x}%`} style:top={`${sparkle.y}%`} style:animation-delay={`${sparkle.delay}s`}>✦</span>{/each}
        </div>{/if}
        <div class="activity-label"><GoalIcon size={14} /><strong>Goal</strong><span class="status">{label(goal.status)}</span></div>
        <details><summary title={goal.objective}>{goal.objective}</summary><p>{goal.objective}</p></details>
        {#if goal.tokensUsed !== undefined}<div class="goal-usage">
          <span>{number(goal.tokensUsed)}{goal.tokenBudget ? ` / ${number(goal.tokenBudget)}` : ''} tokens</span>
          {#if goal.tokenBudget}<progress aria-label="Goal token budget used" value={goal.tokensUsed} max={goal.tokenBudget}></progress>{/if}
        </div>{/if}
        {#if clearError}<p class="goal-error" role="alert">{clearError}</p>{/if}
        <button class="clear-goal" aria-label="Clear goal" title={clearing ? 'Clearing goal…' : 'Clear goal'} disabled={clearing || disabled} aria-busy={clearing} onclick={onclear}><Trash2 size={14}/></button>
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
  .task-activity { flex: none; min-height: 0; max-height: 25%; overflow: auto; overscroll-behavior: contain; margin: 10px 20px 0; font-size: calc(12px * var(--interface-font-ratio, 1)); }
  section { border: 1px solid var(--line); background: var(--panel); border-radius: 8px; padding: 10px 12px; }
  section + section { margin-top: 8px; }
  .goal-card { position:relative; isolation:isolate; }
  .goal-card.clearable { padding-right: 48px; min-height: 64px; }
  .clear-goal { position:absolute; right:8px; bottom:8px; padding:6px; color:var(--muted); background:transparent; }
  .clear-goal:disabled { opacity:.45; cursor:wait; }
  .goal-error { margin:7px 0 0; color:var(--danger, var(--accent-ink)); }
  .goal-card.in-progress { background:color-mix(in srgb,var(--accent) 6%,var(--panel)); border-color:color-mix(in srgb,var(--accent) 45%,var(--line)); }
  .goal-sparkles { position:absolute; inset:0; z-index:-1; overflow:hidden; border-radius:inherit; pointer-events:none; }
  .goal-sparkles span { position:absolute; font-size:8px; line-height:1; color:var(--accent-ink); text-shadow:0 0 7px var(--accent); animation:goal-twinkle 2.6s ease-in-out infinite; }
  @keyframes goal-twinkle { 0%,100% { opacity:.12; transform:translate(-50%,-50%) scale(.5); } 50% { opacity:.85; transform:translate(-50%,-50%) scale(1); } }
  @media (prefers-reduced-motion:reduce) { .goal-sparkles span { animation:none; opacity:.45; transform:translate(-50%,-50%); } }
  .activity-label { display: flex; align-items: center; gap: 7px; color: var(--accent-ink); }
  strong { font-weight: 600; }
  .status { margin-left: auto; font: calc(10px * var(--interface-font-ratio, 1)) var(--mono); color: var(--muted); }
  details { margin-top: 7px; }
  summary { cursor: pointer; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  p { line-height: 1.5; overflow-wrap: anywhere; }
  .goal-usage { display: flex; align-items: center; gap: 12px; margin-top: 7px; color: var(--muted); font: calc(10px * var(--interface-font-ratio, 1)) var(--mono); }
  progress { width: 90px; height: 5px; accent-color: var(--accent); }
  .computer-card { display: flex; align-items: center; justify-content: space-between; gap: 10px; }
  .computer-info { min-width: 0; }
  .computer-info p { margin: 5px 0 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; color: var(--muted); }
  button { display: flex; align-items: center; gap: 5px; flex: none; padding: 6px 8px; border: 1px solid var(--line); border-radius: 5px; }
  button:hover { background: var(--soft); }
  @media (max-height: 500px) { .task-activity { margin: 5px 10px 0; max-height: 18%; } section { padding: 7px 9px; } }
</style>
