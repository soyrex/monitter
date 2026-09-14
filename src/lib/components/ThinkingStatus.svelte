<script lang="ts">
  import { motionView } from '$lib/motion';
  import { untrack, type Snippet } from 'svelte';
  import { observeActivityClock } from '$lib/activity-clock';
  import { Brain } from '@lucide/svelte';
  import AnimatedTitle from './AnimatedTitle.svelte';

  let { avatar, running = false, starting = false, startedAt, active = true }: {
    avatar?: Snippet; running?: boolean; starting?: boolean; startedAt?: number; active?: boolean;
  } = $props();
  const labels = [
    'Thinking', 'Pondering', 'Reasoning', 'Stewing', 'Considering',
    'Working through it', 'Exploring options', 'Connecting the dots',
    'Looking closer', 'Deliberating',
  ];
  let label = $state(labels[0]);
  let elapsedSeconds = $state(0);
  let activeOrigin = 0;
  let labelBucket = -1;
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
    const statusActive = running || starting;
    const requestedOrigin = startedAt;
    if (!statusActive) {
      activeOrigin = 0;
      elapsedSeconds = 0;
      labelBucket = -1;
      return;
    }
    const current = Date.now();
    const nextOrigin = validOrigin(requestedOrigin, current);
    if (!activeOrigin || nextOrigin !== activeOrigin) {
      activeOrigin = nextOrigin;
      // A newly active run should visibly begin as "Thinking". Do not rotate
      // the label until it has actually crossed the first five-second bucket.
      label = labels[0];
      labelBucket = 0;
    }
    const update = (now: number) => {
      elapsedSeconds = Math.max(0, Math.floor((now - activeOrigin) / 1_000));
      const nextBucket = Math.floor((now - activeOrigin) / 5_000);
      if (running && !starting && nextBucket > labelBucket) {
        labelBucket = nextBucket;
        untrack(nextLabel);
      }
    };
    update(Date.now());
    // Recalculate from startedAt when a pane returns; elapsed time never pauses.
    if (!active) return;
    return observeActivityClock(update);
  });
</script>

<div class="activity reasoning-pending" aria-label={starting ? 'Getting ready' : label}>
  {#if avatar}{@render avatar()}{:else}<Brain size={14}/>{/if}
  <span class="reasoning-label" use:motionView={{key:starting ? 'Getting ready' : label,y:0,duration:100}}><AnimatedTitle text={starting ? 'Getting ready' : label} active={running && !starting} activeTooltip="In progress"/></span>
  {#if running || starting}<time aria-label="Elapsed time" datetime={`PT${elapsedSeconds}S`}>{elapsed}</time>{/if}
</div>

<style>
  .reasoning-pending { display:grid; grid-template-columns:auto minmax(0,1fr) auto; align-items:center; gap:8px; width:100%; margin:4px 0 10px; padding:6px 0; color:var(--muted); font-size:calc(12px * var(--interface-font-ratio, 1)); }
  .reasoning-label { min-width:0; overflow:hidden; white-space:nowrap; text-overflow:ellipsis; }
  time { min-width:7ch; justify-self:end; color:var(--muted); text-align:right; font:calc(10px * var(--interface-font-ratio, 1)) var(--mono); font-variant-numeric:tabular-nums; }
  @media (max-width:640px) { .reasoning-pending { margin:2px 0 7px; padding:5px 0; } }
</style>
