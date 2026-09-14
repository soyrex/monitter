<script lang="ts">
  import { motionView } from '$lib/motion';
  import { untrack, type Snippet } from 'svelte';
  import { Brain } from '@lucide/svelte';
  import AnimatedTitle from './AnimatedTitle.svelte';

  let { avatar, running = false, starting = false }: {
    avatar?: Snippet; running?: boolean; starting?: boolean;
  } = $props();
  const labels = [
    'Thinking', 'Pondering', 'Reasoning', 'Stewing', 'Considering',
    'Working through it', 'Exploring options', 'Connecting the dots',
    'Looking closer', 'Deliberating',
  ];
  let label = $state(labels[0]);
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
</script>

<div class="activity reasoning-pending" aria-label={starting ? 'Getting ready' : label}>
  {#if avatar}{@render avatar()}{:else}<Brain size={14}/>{/if}
  <span use:motionView={{key:starting ? 'Getting ready' : label,y:0,duration:100}}><AnimatedTitle text={starting ? 'Getting ready' : label} active={running && !starting} activeTooltip="In progress"/></span>
</div>

<style>
  .reasoning-pending { display:flex; align-items:center; gap:8px; margin:4px 0 10px; padding:6px 0; color:var(--muted); font-size:calc(12px * var(--interface-font-ratio, 1)); }
  @media (max-width:640px) { .reasoning-pending { margin:2px 0 7px; padding:5px 0; } }
</style>
