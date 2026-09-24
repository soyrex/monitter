<script lang="ts">
  import type { Snippet } from 'svelte';
  import { chatDateClock, chatDateTimeLabel } from '$lib/chat-dates';

  let { name, createdAt, avatar, children, actions }: {
    name: string;
    createdAt: number;
    avatar?: Snippet;
    children?: Snippet;
    actions?: Snippet;
  } = $props();

  // Parent snapshots replace message objects while streaming. Stabilize the
  // primitive before allocating a Date, otherwise unchanged timestamps rebuild
  // their locale formatters on every snapshot through the parent prop getter.
  const timestampValue = $derived(createdAt);
  const timestamp = $derived(new Date(timestampValue));
  const validTimestamp = $derived(Number.isFinite(timestamp.getTime()));
  const time = $derived(validTimestamp ? chatDateTimeLabel(timestampValue, $chatDateClock) : '');
</script>

<div class="message-meta">
  {@render avatar?.()}
  <span class="message-author" title={name}>{name}</span>
  {@render children?.()}
  {@render actions?.()}
  {#if validTimestamp}<time datetime={timestamp.toISOString()} title={timestamp.toLocaleString()}>{time}</time>{/if}
</div>

<style>
  .message-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 0 4px;
    min-width: 0;
    color: var(--ink);
    font-family: var(--interface-font, "IBM Plex Sans", system-ui, sans-serif);
    font-size: calc(11px * var(--interface-font-ratio, 1));
    font-weight: 600;
    line-height: 1.3;
  }
  .message-author {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  time {
    flex: none;
    margin-left: auto;
    color: var(--muted);
    font-family: var(--mono, "IBM Plex Mono", ui-monospace, monospace);
    font-size: calc(10px * var(--interface-font-ratio, 1));
    font-weight: 400;
    line-height: 1.3;
    white-space: nowrap;
  }
</style>
