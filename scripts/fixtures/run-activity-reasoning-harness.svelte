<script lang="ts">
  import { onMount } from 'svelte';
  import RunActivity from '../../src/lib/components/RunActivity.svelte';
  import type { RunEvent } from '../../src/lib/types';

  let running = $state(false);
  let detail = $state('{"content":[],"summary":[],"type":"reasoning"}');
  const event = $derived<RunEvent>({
    id: 'reasoning-1', taskId: 'task-1', kind: 'reasoning', title: 'Reasoning', detail, createdAt: 1,
  });

  onMount(() => {
    (window as Window & { __REASONING_QA__?: Record<string, () => void> }).__REASONING_QA__ = {
      activate: () => { running = true; },
      deactivate: () => { running = false; },
      summary: () => { detail = '{"type":"reasoning","summary":[{"type":"summary_text","text":"I checked the source and found the relevant path."}]}'; },
    };
    return () => { delete (window as Window & { __REASONING_QA__?: unknown }).__REASONING_QA__; };
  });
</script>

<main><RunActivity {event} {running}/></main>

<style>
  :global(html, body, #app) { margin: 0; }
  main { padding: 20px; --panel: #fff; --line: #ddd; --muted: #667; --ink: #171717; --accent-ink: #056; --interface-font-ratio: 1; --mono: ui-monospace, monospace; }
</style>
