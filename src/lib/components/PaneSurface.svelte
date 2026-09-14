<script lang="ts">
  import { type Snippet } from 'svelte';

  /**
   * The shared visual surface for root and split panes. It deliberately owns
   * only DOM-local work: inactive splits retain their rendered state but stop
   * their resize observer, so root-only AppSurface coordination never needs a
   * second observer per hidden pane.
   */
  let { active = true, contentKey, compactTabs = false, autoHideTabs = false, modernTabs = false, focusStep = 0, mobileSidebar = false, onmetrics, children }: {
    active?: boolean;
    contentKey: string;
    compactTabs?: boolean;
    autoHideTabs?: boolean;
    modernTabs?: boolean;
    focusStep?: number;
    mobileSidebar?: boolean;
    onmetrics: (width: number) => void;
    children: Snippet;
  } = $props();
  let node = $state<HTMLDivElement>();
  export function element() { return node; }

  $effect(() => {
    if (!node || !active) return;
    const resize = new ResizeObserver(() => {
      if (!node?.isConnected || node.clientWidth <= 0) return;
      onmetrics(node.clientWidth);
    });
    resize.observe(node);
    onmetrics(node.clientWidth);
    return () => resize.disconnect();
  });
</script>

<div bind:this={node} class="pane-surface" data-pane-active={active} data-pane-content={contentKey} data-pane-compact={compactTabs} data-pane-autohide={autoHideTabs} data-pane-modern={modernTabs} data-pane-expansion={focusStep} data-pane-mobile={mobileSidebar}>
  {@render children()}
</div>

<style>
  /* This is a real pane boundary: it owns sizing and is the stable motion
     target for both the main pane and retained split panes. Flattening this
     node would discard that boundary and reintroduce parent-wide observation. */
  .pane-surface { display:flex; flex:1 1 auto; min-width:0; min-height:0; overflow:hidden; }
  .pane-surface > :global(.workspace) { flex:1 1 auto; min-width:0; min-height:0; }
</style>
