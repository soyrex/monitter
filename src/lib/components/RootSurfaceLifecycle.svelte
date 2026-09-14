<script lang="ts">
  import { onMount } from 'svelte';

  /**
   * Root-only bridge, native-drop and workspace-persistence lifecycle.
   *
   * This component is mounted only by the root AppSurface. Split panes do not
   * even register these listeners; they receive the root snapshot and route
   * local state changes through their PaneSurface handle instead.
   */
  let { start }: { start: () => void | (() => void) } = $props();
  onMount(start);
</script>
