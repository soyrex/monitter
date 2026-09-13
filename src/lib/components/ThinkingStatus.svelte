<script lang="ts">
  import { untrack, type Snippet } from 'svelte';
  import { Brain } from '@lucide/svelte';

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
  <span>{starting ? 'Getting ready' : label}</span>
</div>

<style>
  .reasoning-pending { display:flex; align-items:center; gap:8px; margin:12px 0 28px; padding:8px 0; color:var(--muted); font-size:calc(12px * var(--interface-font-ratio, 1)); }
</style>
