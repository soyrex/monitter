<script lang="ts">
  import { motionView } from '$lib/motion';
  import { untrack, type Snippet } from 'svelte';
  import { Brain } from '@lucide/svelte';
  import AnimatedTitle from './AnimatedTitle.svelte';

  let { avatar, running = false, starting = false, startedAt }: {
    avatar?: Snippet; running?: boolean; starting?: boolean; startedAt?: number;
  } = $props();
  const labels = [
    'Thinking', 'Pondering', 'Reasoning', 'Stewing', 'Considering',
    'Working through it', 'Exploring options', 'Connecting the dots',
    'Looking closer', 'Deliberating',
  ];
  let label = $state(labels[0]);
  let elapsedSeconds = $state(0);
  let activeOrigin = 0;
  const validOrigin = (value: number | undefined, current: number) =>
    typeof value === 'number' && Number.isFinite(value) && value >= 946_684_800_000 && value <= current
      ? value
      : current;
  const elapsed = $derived.by(() => {
    if (elapsedSeconds < 60) return `${elapsedSeconds}s`;
    const minutes = Math.floor(elapsedSeconds / 60);
    const seconds = elapsedSeconds % 60;
    if (minutes < 60) return `${minutes}m ${seconds}s`;
    return `${Math.floor(minutes / 60)}h ${minutes % 60}m ${seconds}s`;
  });
  function nextLabel() {
    const offset = 1 + Math.floor(Math.random() * (labels.length - 1));
    label = labels[(labels.indexOf(label) + offset) % labels.length];
  }
  $effect(() => {
    if (!running || starting) return;
    untrack(nextLabel);
    const timer = window.setInterval(nextLabel, 5_000);
    return () => window.clearInterval(timer);
  });
  $effect(() => {
    const active = running || starting;
    const requestedOrigin = startedAt;
    if (!active) {
      activeOrigin = 0;
      elapsedSeconds = 0;
      return;
    }
    const current = Date.now();
    if (!activeOrigin || (requestedOrigin !== undefined && validOrigin(requestedOrigin, current) !== activeOrigin)) {
      activeOrigin = validOrigin(requestedOrigin, current);
    }
    const update = () => { elapsedSeconds = Math.max(0, Math.floor((Date.now() - activeOrigin) / 1_000)); };
    update();
    const timer = window.setInterval(update, 1_000);
    return () => window.clearInterval(timer);
  });
</script>

<div class="activity reasoning-pending" aria-label={starting ? 'Getting ready' : label}>
  {#if avatar}{@render avatar()}{:else}<Brain size={14}/>{/if}
  <span use:motionView={{key:starting ? 'Getting ready' : label,y:0,duration:100}}><AnimatedTitle text={starting ? 'Getting ready' : label} active={running && !starting} activeTooltip="In progress"/></span>
  {#if running || starting}<time aria-label="Elapsed time" datetime={`PT${elapsedSeconds}S`}>{elapsed}</time>{/if}
</div>

<style>
  .reasoning-pending { display:flex; align-items:center; gap:8px; margin:4px 0 10px; padding:6px 0; color:var(--muted); font-size:calc(12px * var(--interface-font-ratio, 1)); }
  .reasoning-pending > span { min-width:0; }
  time { flex:none; margin-left:auto; color:var(--muted); font:calc(10px * var(--interface-font-ratio, 1)) var(--mono); font-variant-numeric:tabular-nums; }
  @media (max-width:640px) { .reasoning-pending { margin:2px 0 7px; padding:5px 0; } }
</style>
