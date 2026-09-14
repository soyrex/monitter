<script lang="ts">
  import { Bot, CircleCheck, CircleStop, Play, Send } from '@lucide/svelte';
  import type { Agent, Collaboration } from '$lib/types';
  let { collaboration, agent, eventTitle = '', steered = false, onclick }: { collaboration: Collaboration; agent?: Agent; eventTitle?: string; steered?: boolean; onclick?: () => void } = $props();
  const name = $derived(agent?.name ?? 'Subagent');
  const phase = $derived.by(() => {
    const title = eventTitle.toLowerCase();
    if (collaboration.kind === 'message') return { label: steered ? `Steered ${name}` : title.includes('started') ? `Message delivered to ${name}` : `Messaged ${name}`, icon: Send };
    if (title.includes('queued')) return { label: `Spawned ${name}`, icon: Bot };
    if (title.includes('started')) return { label: `${name} started`, icon: Play };
    if (title.includes('interrupted')) return { label: `${name} stopped`, icon: CircleStop };
    if (title.includes('error')) return { label: `${name} failed`, icon: CircleStop };
    if (title.includes('completed')) return { label: `${name} finished`, icon: CircleCheck };
    return { label: collaboration.status === 'running' ? `${name} is running` : collaboration.status === 'queued' ? `Spawned ${name}` : `${name} ${collaboration.status}`, icon: Bot };
  });
  const Icon = $derived(phase.icon);
  const terminalEvent = $derived(!eventTitle || /completed|interrupted|error/i.test(eventTitle));
  const summary = $derived(terminalEvent ? collaboration.error || collaboration.result || collaboration.text : collaboration.text);
</script>

<button class="subagent-event" class:running={collaboration.status === 'running'} class:failed={collaboration.status === 'error'} onclick={onclick}>
  <span class="event-icon"><Icon size={14}/></span>
  <span><strong>{phase.label}</strong><small>{summary}</small></span>
</button>

<style>
  .subagent-event{display:flex;align-items:flex-start;gap:9px;width:100%;margin:8px 0 16px;padding:9px 11px;border:1px solid color-mix(in srgb,var(--accent) 24%,var(--line));border-radius:8px;background:color-mix(in srgb,var(--accent) 5%,var(--panel));color:var(--ink);text-align:left}.event-icon{display:grid;place-items:center;width:25px;height:25px;flex:none;border-radius:7px;background:color-mix(in srgb,var(--accent) 13%,var(--soft));color:var(--accent-ink)}.subagent-event>span:last-child{display:grid;min-width:0;gap:3px}.subagent-event strong{font-size:calc(11.5px * var(--interface-font-ratio,1));font-weight:600}.subagent-event small{overflow:hidden;color:var(--muted);font-size:calc(10.5px * var(--interface-font-ratio,1));text-overflow:ellipsis;white-space:nowrap}.subagent-event.running .event-icon{animation:pulse 1.5s ease-in-out infinite}.subagent-event.failed{border-color:color-mix(in srgb,var(--danger,#c44c79) 35%,var(--line))}@keyframes pulse{50%{opacity:.45}}@media(prefers-reduced-motion:reduce){.subagent-event.running .event-icon{animation:none}}
</style>
