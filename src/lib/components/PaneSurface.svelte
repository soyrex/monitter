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
  let observed = $state<HTMLElement>();
  export function element() { return observed; }

  $effect(() => {
    observed = node?.firstElementChild instanceof HTMLElement ? node.firstElementChild : undefined;
    if (!observed || !active) return;
    const resize = new ResizeObserver(() => {
      if (!observed?.isConnected || observed.clientWidth <= 0) return;
      onmetrics(observed.clientWidth);
    });
    resize.observe(observed);
    onmetrics(observed.clientWidth);
    return () => resize.disconnect();
  });
</script>

<div bind:this={node} class="pane-surface" data-pane-active={active} data-pane-content={contentKey} data-pane-compact={compactTabs} data-pane-autohide={autoHideTabs} data-pane-modern={modernTabs} data-pane-expansion={focusStep} data-pane-mobile={mobileSidebar}>
  {@render children()}
</div>

<style>.pane-surface{display:contents}</style>
