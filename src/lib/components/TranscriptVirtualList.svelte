<script lang="ts" generics="T">
  import { createVirtualizer, defaultRangeExtractor, type VirtualizerOptions } from '@tanstack/svelte-virtual';
  import { flushSync, untrack, type Snippet } from 'svelte';
  import { useTranscriptScrollController } from '$lib/transcript-scroll-owner';

  /** Bounded renderer with one virtualizer owning transcript geometry and scrolling. */
  let { items, getKey, children, footer, estimateHeight = 120, overscan = 6, stickyKey, active = true }: {
    items: T[];
    getKey: (item: T, index: number) => string;
    children: Snippet<[item: T, index: number]>;
    /** Height-contributing content after the rows, such as thinking and channel waiting states. */
    footer?: Snippet;
    estimateHeight?: number;
    overscan?: number;
    /** Kept mounted by callers for their existing sticky CSS. */
    stickyKey?: string | null;
    active?: boolean;
  } = $props();

  let root = $state<HTMLDivElement>();
  let footerElement = $state<HTMLDivElement>();
  let scrollParent: HTMLElement | null = null;
  let footerHeight = $state(0);
  let scrollMargin = $state(0);
  let footerObserver: ResizeObserver | undefined;
  let unregisterOwner: (() => void) | undefined;
  const controller = useTranscriptScrollController();

  const virtualizer = createVirtualizer<HTMLElement, HTMLDivElement>({
    count: 0,
    getScrollElement: () => scrollParent,
    estimateSize: () => estimateHeight,
    getItemKey: index => getKey(items[index], index),
    overscan,
    scrollMargin: 0,
    paddingEnd: 0,
    // Chat is end-anchored: prepending preserves the reader's row and a
    // streamed final row remains pinned only for an already-following reader.
    anchorTo: 'end',
    followOnAppend: 'instant',
    scrollEndThreshold: 50,
    // The adapter publishes before this callback. Flush the measured range
    // before core applies a synchronous end-anchor adjustment.
    onChange: (_instance, sync) => { if (sync) flushSync(); },
  });

  function setVirtualizerOptions(options: Partial<VirtualizerOptions<HTMLElement, HTMLDivElement>>) {
    // The Svelte adapter publishes on every setOptions call. Effects that both
    // read its store and write options loop forever, so option writes are
    // deliberately untracked and keyed only by component inputs.
    untrack(() => $virtualizer.setOptions(options));
  }

  function updateFooterHeight() {
    const next = Math.ceil(footerElement?.getBoundingClientRect().height ?? 0);
    if (next === footerHeight) return;
    footerHeight = next;
    setVirtualizerOptions({ paddingEnd: next });
    // Footer measurements use the same virtualizer-owned scroll write.
    if (controller && controller.isFollowing()) $virtualizer.scrollToEnd({ behavior: 'instant' });
  }

  function measureRow(node: HTMLDivElement) {
    $virtualizer.measureElement(node);
  }

  $effect(() => {
    const stickyIndex = stickyKey ? items.findIndex((item, index) => getKey(item, index) === stickyKey) : -1;
    setVirtualizerOptions({
      count: items.length,
      estimateSize: () => estimateHeight,
      getItemKey: index => getKey(items[index], index),
      overscan,
      scrollMargin,
      paddingEnd: footerHeight,
      rangeExtractor: range => {
        const indexes = defaultRangeExtractor(range);
        return stickyIndex >= 0 && !indexes.includes(stickyIndex) ? [...indexes, stickyIndex].sort((a, b) => a - b) : indexes;
      },
      // A deliberate upward read disables both append follow and end anchoring;
      // being within the threshold must never silently recapture that reader.
      anchorTo: controller?.isFollowing() === false ? 'start' : 'end',
      followOnAppend: controller?.isFollowing() === false ? false : 'instant',
    });
  });

  $effect(() => {
    if (!root) return;
    scrollParent = root.closest<HTMLElement>('.messages');
    setVirtualizerOptions({ getScrollElement: () => scrollParent });
    const measureMargin = () => {
      if (!scrollParent) return;
      scrollMargin = root.getBoundingClientRect().top - scrollParent.getBoundingClientRect().top + scrollParent.scrollTop;
    };
    measureMargin();
    const observer = new ResizeObserver(measureMargin);
    observer.observe(root);
    if (scrollParent) observer.observe(scrollParent);
    unregisterOwner = controller?.register({
      scrollToLatest: () => $virtualizer.scrollToEnd({ behavior: 'instant' }),
      isAtLatest: () => $virtualizer.isAtEnd(50),
      setFollowing: following => setVirtualizerOptions({ anchorTo: following ? 'end' : 'start', followOnAppend: following ? 'instant' : false }),
    });
    return () => { observer.disconnect(); unregisterOwner?.(); unregisterOwner = undefined; scrollParent = null; };
  });

  $effect(() => {
    if (!footerElement) return;
    footerObserver = new ResizeObserver(updateFooterHeight);
    footerObserver.observe(footerElement);
    updateFooterHeight();
    return () => { footerObserver?.disconnect(); footerObserver = undefined; };
  });

</script>

{@const rows = $virtualizer.getVirtualItems().toSorted((left, right) => left.index - right.index)}
<div class="transcript-virtual-list" bind:this={root} role="feed" aria-busy={!active} aria-label="Conversation transcript">
  {#if rows.length}<div aria-hidden="true" style:height={`${Math.max(0, rows[0].start - scrollMargin)}px`}></div>{/if}
  {#each rows as row, index (row.key)}
    {@const item = items[row.index]}
    {#if index > 0}<div aria-hidden="true" style:height={`${Math.max(0, row.start - rows[index - 1].end)}px`}></div>{/if}
    <div class="transcript-row" data-index={row.index} use:measureRow>
      {@render children(item, row.index)}
    </div>
  {/each}
  {@const renderedEnd = rows.length ? rows.at(-1)!.end - scrollMargin : 0}
  <div aria-hidden="true" style:height={`${Math.max(0, $virtualizer.getTotalSize() - footerHeight - renderedEnd)}px`}></div>
  {#if footer}<div class="transcript-footer" bind:this={footerElement}>{@render footer()}</div>{/if}
</div>

<style>
  .transcript-virtual-list,.transcript-row,.transcript-footer { min-width:0; }
  .transcript-row { overflow-anchor:none; }
</style>
