<script lang="ts">
  import { onMount } from 'svelte';
  import type { Goal, Snapshot, Task } from '$lib/types';
  import { projectWatch, selectWatchBuild, watchGoal } from '$lib/watch';
  import { unifiedSubagentStatus, type UnifiedSubagent } from '$lib/unified-subagents';
  import WorkPlanPanel from './WorkPlanPanel.svelte';
  import Markdown from './Markdown.svelte';
  let { task, snapshot, goal = null, goalNote = '', goalError = '', mobile = false, onOpenChat, onOpenSubagent }: { task: Task; snapshot: Snapshot; goal?: Goal | null; goalNote?: string; goalError?: string; mobile?: boolean; onOpenChat?: () => void; onOpenSubagent?: (id: string) => void } = $props();
  let now = $state(Date.now());
  onMount(() => { const timer = setInterval(() => now = Date.now(), 1000); return () => clearInterval(timer); });
  const view = $derived(projectWatch(snapshot, task));
  const build = $derived(selectWatchBuild(snapshot, task, now));
  const goalView = $derived(goal ? watchGoal(goal, view.activePlan, view.progress) : null);
  const time = (value: number) => new Date(value).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  const status = $derived(task.status === 'running' ? 'Working' : task.status === 'error' ? 'Needs attention' : task.status === 'completed' ? 'Done' : task.status === 'interrupted' ? 'Stopped' : 'Waiting');
</script>

{#snippet teamRow(member: UnifiedSubagent)}
  <button class="team-row" disabled={!onOpenSubagent} onclick={() => onOpenSubagent?.(member.id)}>
    <span class="dot" class:active={member.status === 'running' || member.status === 'queued'}></span>
    <span class="team-copy"><strong>{member.title}</strong><span>{member.error || member.result || member.activity || member.summary || member.agentName}</span></span>
    <small>{unifiedSubagentStatus(member.status)}</small>
  </button>
{/snippet}

<section class="watch-pane" class:mobile aria-label={`Watch ${task.title}`}>
  <header class="watch-header"><div><h2>{task.title || 'Untitled chat'}</h2><p>{status} · {task.provider}{task.model ? ` · ${task.model}` : ''}</p></div>{#if onOpenChat}<button onclick={onOpenChat}>Open chat</button>{/if}</header>
  {#if mobile}
    {#if build}
      <div class="watch-pills build-status" aria-label="Current build status">
        <div class="watch-pill build-pill" class:attention={build.stale} role="status">
          {#if build.run && !build.stale}<span class="build-spinner" aria-hidden="true"></span>{/if}
          <span class="pill-label">Build</span>
          <strong>{build.label}</strong>
          <span class="pill-detail">{build.title}</span>
        </div>
      </div>
    {/if}
    {#if view.activePlan}
      <section class="mobile-plan" aria-label="Current work plan">
        <WorkPlanPanel plans={[view.activePlan]} collapseDetails/>
      </section>
    {/if}
    {#if goal && goalView}
      <div class="watch-pills goal-status" aria-label="Current goal">
        <div class="watch-pill goal-pill" class:active={goal.status === 'active'} data-testid="watch-goal">
          <span class="pill-label">Goal</span>
          <strong>{goal.objective}</strong>
          {#if goalView.current}<span class="pill-detail">{goalView.current}</span>{/if}
        </div>
      </div>
    {:else if goalNote}
      <div class="watch-pills goal-status" aria-label="Current goal">
        <div class="watch-pill goal-pill"><span class="pill-label">Goal</span><strong>{goalNote}</strong></div>
      </div>
    {/if}
    <section class="mobile-activity" aria-label="Activity stream">
      <div class="section-heading"><h3>Activity</h3><span>{view.activity.length} updates</span></div>
      <div class="activity-stream">
        {#if view.progress}
          <div class="activity activity-current">
            <span><small>Now</small>{view.progress.title}</span>
            <time datetime={new Date(view.progress.createdAt).toISOString()}>{time(view.progress.createdAt)}</time>
          </div>
        {/if}
        {#each view.activity.filter(event => event.id !== view.progress?.id) as event (event.id)}
          <div class="activity">
            <span><small>{event.kind.replaceAll('_', ' ')}</small>{event.title || event.kind}</span>
            <time datetime={new Date(event.createdAt).toISOString()}>{time(event.createdAt)}</time>
          </div>
        {:else}
          {#if !view.progress}<p class="empty">No activity for this request yet.</p>{/if}
        {/each}
      </div>
    </section>
  {:else}
  {#if build}<div class="build" class:attention={build.stale} role="status">{#if build.run && !build.stale}<span class="build-spinner" aria-hidden="true"></span>{/if}{build.label} · {build.title}</div>{/if}
  {#if goal && goalView}
    <section class="goal" class:active={goal.status === 'active'} aria-label="Goal" data-testid="watch-goal">
      <h3>{goalView.label}</h3><p>{goal.objective}</p>
      {#if view.activePlan}<small>Current plan · {goalView.completed} / {goalView.total} steps complete · {view.activePlan.title}</small>{/if}
      {#if goalView.current}<p class="goal-step">{goalView.current}</p>{/if}
      {#if goalView.usage}<small>{goalView.usage}</small>{/if}
      {#if goalError}<small role="status">Showing last known goal · update unavailable</small>{/if}
    </section>
  {:else if goalNote}<p class="goal-note">Last goal update · {goalNote}</p>{:else if goalError}<p class="goal-note" role="status">Goal update unavailable</p>{/if}
  <section class="now" aria-label="Current update"><p>{view.progress?.title || (task.status === 'running' ? view.activePlan ? 'Working through the current plan' : 'Working on the current request' : 'No active update')}</p>{#if view.progress}<time datetime={new Date(view.progress.createdAt).toISOString()}>{time(view.progress.createdAt)}</time>{/if}<div class="live-line" class:running={task.status === 'running'}></div></section>
  {#if view.activePlan}<WorkPlanPanel plans={[view.activePlan]}/>{:else}<section class="task-summary"><h3>Current request</h3>{#if view.request}<div class="request"><Markdown text={view.request.text} taskId={task.id}/></div>{:else}<p>No request available for this chat yet.</p>{/if}{#if view.finishedPlan}<details><summary>Last plan · {view.finishedPlan.title}</summary><WorkPlanPanel plans={[view.finishedPlan]}/></details>{/if}</section>{/if}
  <section aria-label="Subagents"><div class="section-heading"><h3>Subagents</h3><span>{view.activeTeam.length} running · {view.finishedTeam.length} done</span></div>{#each view.activeTeam as member (member.id)}{@render teamRow(member)}{/each}{#if !view.team.length}<p class="empty">No subagents for this work yet.</p>{/if}{#if view.finishedTeam.length}<details><summary>{view.finishedTeam.length} finished</summary>{#each view.finishedTeam as member (member.id)}{@render teamRow(member)}{/each}</details>{/if}</section>
  {#if view.tool}<div class="latest-tool"><span>{view.tool.title}</span><time datetime={new Date(view.tool.createdAt).toISOString()}>{time(view.tool.createdAt)}</time></div>{/if}
  <section aria-label="Recent activity"><h3>Recent activity</h3>{#each view.activity as event (event.id)}<div class="activity"><span>{event.title || event.kind}</span><time datetime={new Date(event.createdAt).toISOString()}>{time(event.createdAt)}</time></div>{:else}<p class="empty">No activity for this request yet.</p>{/each}</section>
  {/if}
</section>

<style>
  .goal{display:grid;gap:8px;padding:14px;border:1px solid var(--line);border-radius:10px;background:var(--panel)}.goal.active{border-color:var(--accent)}.goal h3,.goal p{margin:0}.goal p{font-size:14px;line-height:1.5;white-space:pre-wrap}.goal.active h3{color:var(--accent)}.goal small,.goal-note{font-size:12px;line-height:1.5;color:var(--muted)}.goal .goal-step{font-size:13px}
  .watch-pane.mobile{max-width:none;padding:16px;gap:18px}.watch-pills{display:flex;flex-wrap:wrap;gap:8px}.watch-pill{display:flex;align-items:center;gap:7px;max-width:100%;min-height:30px;padding:5px 10px;border:1px solid var(--line);border-radius:999px;background:var(--panel);font-size:11px;line-height:18px;box-shadow:0 2px 8px #0000000d}.watch-pill.active{border-color:color-mix(in srgb,var(--accent) 55%,var(--line))}.watch-pill .pill-label{flex:none;text-transform:uppercase;letter-spacing:.06em;font-size:9px;font-weight:700;color:var(--muted)}.watch-pill strong{min-width:0;max-width:36ch;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:11px;font-weight:600}.watch-pill .pill-detail{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--muted)}.goal-pill.active .pill-label,.build-pill{color:var(--accent)}.build-pill.attention{color:var(--danger,#c96058)}.mobile-activity{min-width:0}.activity-stream{border-top:1px solid var(--line)}.mobile .activity{align-items:center;min-height:44px;padding:8px 0}.mobile .activity>span{display:grid;gap:2px;font-size:13px;line-height:17px}.mobile .activity small{font-size:9px;line-height:12px;text-transform:uppercase;letter-spacing:.06em;color:var(--muted)}.mobile .activity-current>span{font-weight:600}.mobile .activity-current small{color:var(--accent)}
  .watch-pane{box-sizing:border-box;display:flex;flex-direction:column;gap:22px;min-width:0;width:100%;max-width:900px;margin:0 auto;padding:20px;color:var(--ink);overflow-wrap:anywhere}.watch-pane section,.watch-header>div{min-width:0}.watch-header,.section-heading{display:flex;align-items:center;justify-content:space-between;gap:12px}.watch-header h2{font-size:16px;margin:0}.watch-header p,.empty{font-size:12px;color:var(--muted);margin:6px 0}.watch-pane h3{font-size:11px;text-transform:uppercase;letter-spacing:.06em;color:var(--muted);margin:0 0 10px}.watch-pane button,.watch-pane summary{min-height:44px;box-sizing:border-box}.watch-header button{flex:none;border:1px solid var(--line);border-radius:8px;padding:8px 12px;color:var(--ink);background:var(--panel)}.now{position:relative;padding-bottom:12px}.now p{font-size:15px;line-height:1.5;margin:0}.now time{display:block;margin-top:5px}.live-line{height:2px;margin-top:12px;border-radius:2px;background:var(--line);overflow:hidden}.live-line.running{background:var(--accent);animation:breathe 1.8s ease-in-out infinite}.task-summary{padding:16px;background:var(--panel);border:1px solid var(--line);border-radius:12px}.request{font-size:14px;line-height:1.6;max-height:320px;overflow:auto}.section-heading span,time,.team-row small{font-size:11px;color:var(--muted)}.section-heading h3{margin:0}.section-heading{margin-bottom:8px}.team-row{display:flex;align-items:center;gap:10px;width:100%;padding:9px 0;border:0;border-bottom:1px solid var(--line);background:transparent;text-align:left;color:var(--ink)}.team-row:disabled{opacity:1;cursor:default}.team-copy{display:grid;gap:4px;flex:1;min-width:0}.team-copy strong{font-size:13px}.team-copy>span{font-size:12px;color:var(--muted);display:-webkit-box;-webkit-line-clamp:2;line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}.dot{width:7px;height:7px;flex:none;border-radius:50%;background:var(--muted)}.dot.active{background:var(--accent)}.watch-pane details{margin-top:8px}.watch-pane summary{display:flex;align-items:center;cursor:pointer;color:var(--muted);font-size:12px;gap:6px}.watch-pane summary:before{content:'›'}.watch-pane details[open]>summary:before{content:'⌄'}.build{padding:10px 12px;background:color-mix(in srgb,var(--accent) 10%,var(--panel));border:1px solid var(--line);border-radius:8px;font-size:12px;color:var(--accent-ink,var(--accent))}.build-spinner{display:inline-block;vertical-align:-2px;width:11px;height:11px;margin-right:7px;border:2px solid currentColor;border-right-color:transparent;border-radius:50%;animation:spin .9s linear infinite}@keyframes spin{to{transform:rotate(360deg)}}.build.attention{color:var(--danger,#c96058)}.activity,.latest-tool{display:flex;align-items:baseline;justify-content:space-between;gap:12px;padding:9px 0;font-size:12px;border-bottom:1px solid var(--line)}.activity>span,.latest-tool>span{min-width:0}.activity time,.latest-tool time{flex:none}.latest-tool{color:var(--muted)}button:focus-visible,summary:focus-visible{outline:2px solid var(--accent);outline-offset:3px}@keyframes breathe{50%{opacity:.35}}@media(prefers-reduced-motion:reduce){.live-line.running,.build-spinner{animation:none}}@media(max-width:480px){.watch-pane{padding:16px;gap:18px}.watch-header{align-items:flex-start}.watch-header h2{font-size:15px}.watch-header p{font-size:11px}.section-heading{flex-wrap:wrap}}
</style>
