<script lang="ts">
  import RemoteControl from "$lib/components/RemoteControl.svelte";
  import ShareControl from "$lib/components/ShareControl.svelte";
  import AppSurface from "$lib/components/AppSurface.svelte";
  import LanGate from '$lib/components/LanGate.svelte';
  import { isLanBrowser } from '$lib/lan';
</script>
<LanGate>
<AppSurface />

{#if !isLanBrowser()}
<RemoteControl />
<ShareControl />
{/if}
</LanGate>
