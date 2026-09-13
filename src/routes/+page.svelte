<script lang="ts">
  import RemoteControl from "$lib/components/RemoteControl.svelte";
  import ShareControl from "$lib/components/ShareControl.svelte";
  import AppSurface from "$lib/components/AppSurface.svelte";
  import LanGate from '$lib/components/LanGate.svelte';
  import { isLanBrowser } from '$lib/lan';
  import { remoteControlOpen, workspaceShareOpen, workspaceShareTaskId } from '$lib/workspace-panels';
</script>
<LanGate>
<AppSurface />

{#if !isLanBrowser()}
<RemoteControl bind:open={$remoteControlOpen} />
<ShareControl bind:open={$workspaceShareOpen} taskId={$workspaceShareTaskId} />
{/if}
</LanGate>
