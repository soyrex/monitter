<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { getBridge } from '$lib/bridge';
  import type { Snapshot } from '$lib/types';
  import AppSurface from '$lib/components/AppSurface.svelte';
  const bridge = getBridge();
  let snapshot = $state.raw<Snapshot | null>(null);
  let surface = $state<AppSurface>();
  onMount(() => {
    let off: (() => void) | undefined;
    const refresh = async () => { snapshot = await bridge.getSnapshot(); };
    void (async () => {
      await refresh();
      await tick();
      surface!.openTask(snapshot!.tasks[0]);
      off = await bridge.onChanged(refresh);
      (window as any).__appHistory = {
        messages: () => snapshot!.messages,
        refresh,
        page: async (before?: string) => {
          const page = await bridge.getTaskMessages!(snapshot!.tasks[0].id, before, 64);
          await refresh();
          return page;
        },
      };
    })();
    return () => off?.();
  });
</script>
{#if snapshot}<AppSurface bind:this={surface} embedded parentSnapshot={snapshot}/>{/if}
<style>:global(html,body,#app){height:100%;margin:0}</style>
