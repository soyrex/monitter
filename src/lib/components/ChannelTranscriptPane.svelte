<script lang="ts">
  import { tick } from 'svelte';
  import { type Snippet } from 'svelte';
  import type { ChannelMessage, ChannelMessagesPage } from '$lib/types';
  import { captureTranscriptAnchor, mergeChannelMessagesById } from '$lib/ui-sync';
  import MessagePane from '$lib/components/MessagePane.svelte';
  import TranscriptVirtualList from '$lib/components/TranscriptVirtualList.svelte';

  let {
    channelId,
    messages,
    getChannelMessages,
    active = true,
    pendingUpdates = false,
    resetKey,
    onfollowchange,
    children: renderMessage,
    footer: renderFooter,
  }: {
    channelId: string;
    messages: ChannelMessage[];
    getChannelMessages?: (channelId: string, beforeId?: string, limit?: number) => Promise<ChannelMessagesPage>;
    active?: boolean;
    pendingUpdates?: boolean;
    resetKey: string;
    onfollowchange?: (following: boolean) => void;
    children: Snippet<[ChannelMessage, number, ChannelMessage | undefined]>;
    footer?: Snippet;
  } = $props();

  let historyOwner = $state<string | null>(null);
  let olderMessages = $state<ChannelMessage[]>([]);
  let nextBeforeId = $state<string | null | undefined>();
  let loading = $state(false);
  let error = $state('');
  let paneRoot = $state<HTMLElement>();
  let messagePane: { detachFromLatest: () => void } | undefined = $state();
  let transcriptList: { restoreItemAnchor: (key: string, offset: number) => Promise<boolean> } | undefined = $state();

  $effect(() => {
    if (historyOwner === channelId) return;
    historyOwner = channelId;
    olderMessages = [];
    nextBeforeId = undefined;
    loading = false;
    error = '';
  });

  const hasEarlierMessages = $derived(!!getChannelMessages &&
    (nextBeforeId === undefined ? messages.length >= 64 : nextBeforeId !== null));
  const displayedMessages = $derived(mergeChannelMessagesById(messages, olderMessages));

  function handleFollowChange(following: boolean) {
    onfollowchange?.(following);
    // The bridge cache also retains every successfully loaded page. Removing
    // this detached-only overlay on release makes the latest cache authoritative.
    if (following && olderMessages.length) olderMessages = [];
  }

  async function loadEarlierMessages() {
    if (!getChannelMessages || loading || !hasEarlierMessages) return;
    const requestChannelId = channelId;
    const beforeId = nextBeforeId ?? messages[0]?.id;
    if (!beforeId) { nextBeforeId = null; return; }
    // Explicitly entering history detaches the reader, even when the sticky
    // pager was clicked while following the newest channel message.
    messagePane?.detachFromLatest();
    loading = true;
    error = '';
    const pane = paneRoot?.querySelector<HTMLElement>('.messages');
    const anchor = captureTranscriptAnchor(pane);
    try {
      const page = await getChannelMessages(requestChannelId, beforeId, 64);
      if (page.channelId !== requestChannelId) throw new Error('Channel history page did not match this channel.');
      if (channelId !== requestChannelId) return;
      olderMessages = mergeChannelMessagesById(olderMessages, page.messages);
      nextBeforeId = page.nextBeforeId === beforeId ? null : page.nextBeforeId;
      if (anchor) {
        await tick();
        if (channelId === requestChannelId && await transcriptList?.restoreItemAnchor(anchor.key, anchor.offset)) {
          // The channel pager can disappear when the final page is reached.
          // Let that header/viewport resize and virtualizer measurement settle,
          // then correct against the same scroll viewport one last time.
          await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
          if (channelId === requestChannelId) {
            const viewport = paneRoot?.querySelector<HTMLElement>('.messages');
            const row = paneRoot?.querySelector<HTMLElement>(`[data-item-key="${CSS.escape(anchor.key)}"]`);
            if (viewport && row) viewport.scrollTop += row.getBoundingClientRect().top - viewport.getBoundingClientRect().top - anchor.offset;
          }
        }
      }
    } catch (reason) {
      if (channelId === requestChannelId) error = reason instanceof Error ? reason.message : String(reason);
    } finally { if (channelId === requestChannelId) loading = false; }
  }
</script>

<div class="channel-transcript-pane" bind:this={paneRoot}>
  <MessagePane bind:this={messagePane} {active} {pendingUpdates} onfollowchange={handleFollowChange} {resetKey}>
    {#snippet header()}
      {#if hasEarlierMessages || error}<div class="channel-history-pager">
        {#if hasEarlierMessages}<button type="button" aria-label="Load earlier channel messages" disabled={loading} onclick={() => void loadEarlierMessages()}>{loading ? 'Loading earlier messages…' : 'Load earlier messages'}</button>{/if}
        {#if error}<span role="status">{error}</span>{/if}
      </div>{/if}
    {/snippet}
    <TranscriptVirtualList bind:this={transcriptList} items={displayedMessages} getKey={message => message.id} {active}>
      {#snippet children(message, index)}{@render renderMessage(message, index, displayedMessages[index - 1])}{/snippet}
      {#snippet footer()}{@render renderFooter?.()}{/snippet}
    </TranscriptVirtualList>
  </MessagePane>
</div>

<style>
  .channel-transcript-pane{display:flex;flex:1;min-width:0;min-height:0}
  .channel-history-pager{display:flex;align-items:center;justify-content:center;gap:10px;padding:5px 8px;background:color-mix(in srgb,var(--paper) 92%,transparent);font-size:calc(10px * var(--interface-font-ratio,1))}
  .channel-history-pager button{padding:5px 10px;border:1px solid var(--line);border-radius:6px;color:var(--muted);background:var(--panel);font:inherit}
  .channel-history-pager button:hover:not(:disabled){color:var(--ink);background:var(--soft)}
  .channel-history-pager button:disabled{opacity:.55}
  .channel-history-pager span{color:#bd655b}
</style>
