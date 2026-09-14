<script lang="ts" generics="T">
  import { onMount, tick, type Snippet } from 'svelte';

  /** Bounded renderer for rich MessagePane rows. */
  let { items, getKey, children, estimateHeight = 120, overscan = 6, stickyKey, active = true }: {
    items: T[];
    getKey: (item: T, index: number) => string;
    children: Snippet<[item: T, index: number]>;
    estimateHeight?: number;
    overscan?: number;
    /** Keeps the latest user request mounted for its existing sticky CSS. */
    stickyKey?: string | null;
    /** Pass the owning pane's selected state. */
    active?: boolean;
  } = $props();

  let root = $state<HTMLDivElement>();
  let scrollParent = $state<HTMLElement>();
  let documentVisible = $state(true);
  let range = $state({ start: 0, end: 0 });
  const heights = new Map<string, number>();
  let resizeObserver: ResizeObserver | undefined;
  let frame: number | undefined;

  const heightFor = (item: T, index: number) => heights.get(getKey(item, index)) ?? estimateHeight;
  const offsetFor = (end: number) => items.slice(0, end).reduce((total, item, index) => total + heightFor(item, index), 0);
  const totalHeight = $derived(offsetFor(items.length));
  const stickyIndex = $derived(stickyKey ? items.findIndex((item, index) => getKey(item, index) === stickyKey) : -1);
  const stickyBeforeWindow = $derived(stickyIndex >= 0 && stickyIndex < range.start);
  const topSpacer = $derived(offsetFor(stickyBeforeWindow ? stickyIndex : range.start));
  const stickyGap = $derived(stickyBeforeWindow ? Math.max(0, offsetFor(range.start) - offsetFor(stickyIndex + 1)) : 0);
  const bottomSpacer = $derived(Math.max(0, totalHeight - offsetFor(range.end)));
  const rendered = $derived(items.slice(range.start, range.end));

  function updateRange() {
    frame = undefined;
    if (!root || !scrollParent || !active || !documentVisible) return;
    const visibleTop = Math.max(0, scrollParent.scrollTop - root.offsetTop);
    const visibleBottom = visibleTop + scrollParent.clientHeight;
    let start = 0, cursor = 0;
    while (start < items.length && cursor + heightFor(items[start], start) < visibleTop) {
      cursor += heightFor(items[start], start);
      start += 1;
    }
    let end = start;
    while (end < items.length && cursor < visibleBottom) cursor += heightFor(items[end], end++);
    start = Math.max(0, start - overscan);
    end = Math.min(items.length, end + overscan);
    range = { start, end };
  }
  function scheduleRange() {
    if (frame !== undefined || !active || !documentVisible) return;
    frame = requestAnimationFrame(updateRange);
  }
  function measured(node: HTMLElement, { key, index }: { key: string; index: number }) {
    node.dataset.transcriptKey = key; node.dataset.transcriptIndex = String(index);
    resizeObserver?.observe(node);
    return { destroy: () => resizeObserver?.unobserve(node) };
  }
  function receiveMeasurements(entries: ResizeObserverEntry[]) {
    let changedAboveViewport = 0;
    for (const entry of entries) {
      const element = entry.target as HTMLElement, key = element.dataset.transcriptKey, index = Number(element.dataset.transcriptIndex);
      if (!key || !Number.isInteger(index)) continue;
      const next = Math.max(1, Math.ceil(entry.borderBoxSize?.[0]?.blockSize ?? entry.contentRect.height));
      const previous = heights.get(key) ?? estimateHeight;
      if (next === previous) continue;
      heights.set(key, next);
      if (index < range.start) changedAboveViewport += next - previous;
    }
    if (changedAboveViewport && scrollParent) scrollParent.scrollTop += changedAboveViewport;
    scheduleRange();
  }

  $effect(() => {
    const keys = new Set(items.map(getKey));
    for (const key of heights.keys()) if (!keys.has(key)) heights.delete(key);
    items; stickyKey; estimateHeight; scheduleRange();
  });
  $effect(() => {
    // An inactive split has no reader. Unmount its rich rows (including any
    // nested activity widgets) while retaining only spacer geometry, then
    // calculate the visible window again when that pane becomes active.
    if (!active || !documentVisible) { range = { start: 0, end: 0 }; return; }
    scheduleRange();
  });
  $effect(() => {
    if (!root || !active || !documentVisible) return;
    scrollParent = root.closest<HTMLElement>('.messages') ?? undefined;
    if (!scrollParent) return;
    resizeObserver = new ResizeObserver(receiveMeasurements);
    const onScroll = () => scheduleRange();
    scrollParent.addEventListener('scroll', onScroll, { passive: true });
    scheduleRange();
    return () => {
      scrollParent?.removeEventListener('scroll', onScroll); resizeObserver?.disconnect(); resizeObserver = undefined;
      if (frame !== undefined) cancelAnimationFrame(frame); frame = undefined;
    };
  });
  onMount(() => {
    const visibilityChanged = () => { documentVisible = document.visibilityState !== 'hidden'; if (documentVisible) void tick().then(scheduleRange); };
    documentVisible = document.visibilityState !== 'hidden';
    document.addEventListener('visibilitychange', visibilityChanged);
    return () => document.removeEventListener('visibilitychange', visibilityChanged);
  });
</script>

<div class="transcript-virtual-list" bind:this={root} role="feed" aria-busy={!active} aria-label="Conversation transcript">
  <div aria-hidden="true" style:height={`${topSpacer}px`}></div>
  {#if stickyBeforeWindow}
    {@const stickyItem = items[stickyIndex]}
    <div class="transcript-row" use:measured={{ key: getKey(stickyItem, stickyIndex), index: stickyIndex }}>
      {@render children(stickyItem, stickyIndex)}
    </div>
    <div aria-hidden="true" style:height={`${stickyGap}px`}></div>
  {/if}
  {#each rendered as item, relativeIndex (getKey(item, range.start + relativeIndex))}
    {@const index = range.start + relativeIndex}
    <div class="transcript-row" use:measured={{ key: getKey(item, index), index }}>
      {@render children(item, index)}
    </div>
  {/each}
  <div aria-hidden="true" style:height={`${bottomSpacer}px`}></div>
</div>

<style>.transcript-virtual-list,.transcript-row{min-width:0}.transcript-row{overflow-anchor:none}</style>
