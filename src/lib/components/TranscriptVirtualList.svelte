<script lang="ts" generics="T">
  import { createVirtualizer, defaultRangeExtractor, elementScroll, type VirtualizerOptions } from '@tanstack/svelte-virtual';
  import { flushSync, tick, untrack, type Snippet } from 'svelte';
  import { get } from 'svelte/store';
  import { useTranscriptScrollController } from '$lib/transcript-scroll-owner';

  /** Bounded renderer with one virtualizer owning transcript geometry and scrolling. */
  let { items, getKey, children, footer, estimateHeight = 120, overscan = 6, keepRecent = 30, stickyKey, active = true }: {
    items: T[];
    getKey: (item: T, index: number) => string;
    children: Snippet<[item: T, index: number]>;
    /** Height-contributing content after the rows, such as thinking and channel waiting states. */
    footer?: Snippet;
    estimateHeight?: number;
    overscan?: number;
    /** Keep the newest rows mounted so switching back to a chat is immediate. */
    keepRecent?: number;
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
  let committingOptions = false;
  let canFlushMeasurements = false;
  let followCommitPending = false;
  // A transcript may stay mounted while its owning chat changes. Keep a
  // stable row key from the current transcript so streaming appends and
  // prepended history retain their measurements, while a replacement chat
  // clears the old virtual range and size cache.
  let transcriptAnchorKey: string | null = null;
  const controller = useTranscriptScrollController();
  const isFollowing = () => controller?.isFollowing() !== false;

  // One owner, one post-commit destination: the actual scrollable bottom.
  // Distance is geometry, never permission to stop following. Coalesce work
  // after Svelte commits row/spacer changes, rather than adding a frame of lag.
  function followCommittedLayout() {
    if (followCommitPending || !isFollowing()) return;
    followCommitPending = true;
    void tick().then(() => {
      followCommitPending = false;
      const viewport = scrollParent;
      if (!isFollowing() || !viewport || !viewport.isConnected || viewport.clientHeight === 0) return;
      const bottom = Math.max(0, viewport.scrollHeight - viewport.clientHeight);
      if (Math.abs(viewport.scrollTop - bottom) > 1) instance().scrollToOffset(bottom, { behavior: 'instant' });
    });
  }

  const onVirtualizerChange = (_instance: unknown, sync: boolean) => {
    // setOptions and initial measurement run during Svelte's own update. A
    // later ResizeObserver delivery is outside that update and must synchronously
    // commit its new range before core applies a scroll adjustment.
    if (sync && !committingOptions && canFlushMeasurements) flushSync();
    followCommittedLayout();
  };

  const virtualizer = createVirtualizer<HTMLElement, HTMLDivElement>({
    count: 0,
    getScrollElement: () => scrollParent,
    estimateSize: () => estimateHeight,
    getItemKey: index => getKey(items[index], index),
    overscan: untrack(() => overscan),
    scrollMargin: 0,
    paddingEnd: 0,
    // Chat is end-anchored: prepending preserves the reader's row and a
    // streamed final row remains pinned only for an already-following reader.
    anchorTo: 'end',
    followOnAppend: 'instant',
    scrollEndThreshold: Number.POSITIVE_INFINITY,
    scrollToFn: (offset, options, owner) => {
      // An earlier end request may still have a reconciliation callback queued
      // when the reader scrolls up. Never let that request take the view back.
      if (!isFollowing() && options.behavior === 'instant') return;
      elementScroll(offset, options, owner);
    },
    // The adapter publishes before this callback. Flush the measured range
    // before core applies a synchronous end-anchor adjustment.
    onChange: onVirtualizerChange,
  });

  const instance = () => get(virtualizer);
  const rows = $derived($virtualizer.getVirtualItems().toSorted((left, right) => left.index - right.index));
  // The virtualizer may publish its previous range for one reactive turn while
  // a held transcript, task switch, or refresh supplies a shorter item list.
  // Never pass one of those stale indexes through to either a caller key or
  // the row snippet.
  const renderedRows = $derived(rows.filter(row =>
    row.index >= 0 && row.index < items.length && row.key === getKey(items[row.index], row.index)
  ));
  const renderedEnd = $derived(renderedRows.length ? renderedRows.at(-1)!.end - scrollMargin : 0);

  function setVirtualizerOptions(options: Partial<VirtualizerOptions<HTMLElement, HTMLDivElement>>) {
    // The Svelte adapter publishes on every setOptions call. Effects that both
    // read its store and write options loop forever, so option writes are
    // deliberately untracked and keyed only by component inputs.
    untrack(() => {
      const previousCommitting = committingOptions;
      committingOptions = true;
      try { instance().setOptions({ ...options, onChange: onVirtualizerChange }); }
      finally { committingOptions = previousCommitting; }
      followCommittedLayout();
    });
  }

  function updateFooterHeight() {
    untrack(() => {
      const next = Math.ceil(footerElement?.getBoundingClientRect().height ?? 0);
      if (next === footerHeight) return;
      footerHeight = next;
      setVirtualizerOptions({ paddingEnd: next });
      // Footer measurements use the same virtualizer-owned scroll write.
      followCommittedLayout();
    });
  }

  function measureRow(node: HTMLDivElement) {
    // This action can run while Svelte is mounting a newly visible range.
    // Its immediate measurement must not force a nested flush.
    const previousCommitting = committingOptions;
    committingOptions = true;
    try { instance().measureElement(node); }
    finally { committingOptions = previousCommitting; }
  }

  $effect(() => {
    const firstItem = items[0];
    const firstKey = firstItem === undefined ? null : getKey(firstItem, 0);
    const transcriptChanged = transcriptAnchorKey !== null &&
      (firstKey === null || !items.some((item, index) => getKey(item, index) === transcriptAnchorKey));
    if (transcriptChanged) instance().measure();
    transcriptAnchorKey = firstKey;

    const stickyIndex = stickyKey ? items.findIndex((item, index) => getKey(item, index) === stickyKey) : -1;
    setVirtualizerOptions({
      count: items.length,
      estimateSize: () => estimateHeight,
      getItemKey: index => {
        const item = items[index];
        return item === undefined ? `stale-row:${index}` : getKey(item, index);
      },
      overscan,
      scrollMargin,
      paddingEnd: footerHeight,
      rangeExtractor: range => {
        const indexes = defaultRangeExtractor(range);
        const recentStart = Math.max(0, items.length - keepRecent);
        for (let index = recentStart; index < items.length; index += 1) {
          if (!indexes.includes(index)) indexes.push(index);
        }
        return stickyIndex >= 0 && !indexes.includes(stickyIndex) ? [...indexes, stickyIndex].sort((a, b) => a - b) : indexes;
      },
      // A deliberate upward read disables both append follow and end anchoring;
      // being within the threshold must never silently recapture that reader.
      anchorTo: controller?.isFollowing() === false ? 'start' : 'end',
      followOnAppend: controller?.isFollowing() === false ? false : 'instant',
      scrollEndThreshold: isFollowing() ? Number.POSITIVE_INFINITY : 1,
    });
  });

  $effect(() => {
    if (!root) return;
    const virtualRoot = root;
    scrollParent = virtualRoot.closest<HTMLElement>('.messages');
    setVirtualizerOptions({ getScrollElement: () => scrollParent });
    const measureMargin = () => {
      if (!scrollParent) return;
      scrollMargin = virtualRoot.getBoundingClientRect().top - scrollParent.getBoundingClientRect().top + scrollParent.scrollTop;
      followCommittedLayout();
    };
    measureMargin();
    const observer = new ResizeObserver(measureMargin);
    observer.observe(virtualRoot);
    // Include outer padding and changes outside the rows, plus viewport height
    // changes when the composer/approval area grows or the column is resized.
    if (virtualRoot.parentElement) observer.observe(virtualRoot.parentElement);
    if (scrollParent) observer.observe(scrollParent);
    unregisterOwner = controller?.register({
      scrollToLatest: followCommittedLayout,
      isAtLatest: () => instance().isAtEnd(1),
      setFollowing: following => setVirtualizerOptions({
        anchorTo: following ? 'end' : 'start',
        followOnAppend: following ? 'instant' : false,
        scrollEndThreshold: following ? Number.POSITIVE_INFINITY : 1,
      }),
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

  queueMicrotask(() => { canFlushMeasurements = true; });

</script>

<div class="transcript-virtual-list" bind:this={root} role="feed" aria-busy={!active} aria-label="Conversation transcript">
  {#if renderedRows.length}<div aria-hidden="true" style:height={`${Math.max(0, renderedRows[0].start - scrollMargin)}px`}></div>{/if}
  {#each renderedRows as row, index (row.key)}
    {@const item = items[row.index]!}
    {#if index > 0}<div aria-hidden="true" style:height={`${Math.max(0, row.start - renderedRows[index - 1].end)}px`}></div>{/if}
    <div class="transcript-row" data-index={row.index} use:measureRow>
      {@render children(item, row.index)}
    </div>
  {/each}
  <div aria-hidden="true" style:height={`${Math.max(0, $virtualizer.getTotalSize() - footerHeight - renderedEnd)}px`}></div>
  {#if footer}<div class="transcript-footer" bind:this={footerElement}>{@render footer()}</div>{/if}
</div>

<style>
  .transcript-virtual-list,.transcript-row,.transcript-footer { min-width:0; }
  .transcript-row,.transcript-footer { display:flow-root; }
  .transcript-row { overflow-anchor:none; }
</style>
