<script lang="ts">
  import { onMount } from 'svelte';
  import type { Agent, Message, Snapshot, Task } from '$lib/types';
  import { getBridge } from '$lib/bridge';
  import { captureTranscriptAnchor, mergeOlderTranscriptMessages } from '$lib/ui-sync';
  import { createTranscriptBuffer } from '$lib/transcript-buffer.svelte';
  import MessagePane from '$lib/components/MessagePane.svelte';
  import TranscriptVirtualList from '$lib/components/TranscriptVirtualList.svelte';
  import ChannelTranscriptPane from '$lib/components/ChannelTranscriptPane.svelte';

  const taskId = '11111111-1111-4111-8111-111111111111';
  const bridge = getBridge();
  let snapshot = $state<Snapshot>({ hosts: [], agents: [], tasks: [], messages: [], events: [], channels: [], projects: [], collaborations: [], queuedMessages: [], approvalRequests: [], approvalRules: [], settings: { accent: '#3978d4', theme: 'light', interfaceScale: 100, showToolActivity: true, showReasoningSummaries: true, sendWithEnter: false, sidebarView: 'standard' } });
  let olderMessages = $state<Message[]>([]);
  let nextBeforeId = $state<string | null | undefined>();
  let loading = $state(false);
  let channelMode = $state(false);
  let selectedChannelId = $state('kept-channel');
  let transcriptList: { restoreItemAnchor: (key: string, offset: number) => Promise<boolean> } | undefined = $state();
  const buffer = createTranscriptBuffer(() => taskId, () => ({ messages: snapshot.messages }), () => JSON.stringify(snapshot.messages.map(message => [message.id, message.text])));
  const displayed = $derived(buffer.value());
  const messages = $derived(mergeOlderTranscriptMessages(displayed.messages.filter(message => message.taskId === taskId), olderMessages));
  const channelBuffer = createTranscriptBuffer(() => selectedChannelId, () => ({ messages: snapshot.channels.find(channel => channel.id === selectedChannelId)?.messages ?? [] }), () => JSON.stringify(snapshot.channels.find(channel => channel.id === selectedChannelId)?.messages.map(message => [message.id, message.text]) ?? []));
  const displayedChannel = $derived(channelBuffer.value());
  const canLoadEarlier = $derived(nextBeforeId === undefined ? displayed.messages.filter(message => message.taskId === taskId).length >= 64 : nextBeforeId !== null);

  async function refresh() { snapshot = await bridge.getSnapshot(); }
  async function loadEarlier() {
    if (!bridge.getTaskMessages || loading || !canLoadEarlier) return;
    const visible = messages;
    const beforeId = nextBeforeId ?? visible[0]?.id;
    if (!beforeId) { nextBeforeId = null; return; }
    loading = true;
    const viewport = document.querySelector<HTMLElement>('.messages');
    const anchor = buffer.held() ? captureTranscriptAnchor(viewport) : null;
    try {
      const page = await bridge.getTaskMessages(taskId, beforeId, 16);
      olderMessages = mergeOlderTranscriptMessages(olderMessages, page.messages);
      nextBeforeId = page.nextBeforeId === beforeId ? null : page.nextBeforeId;
      if (anchor) await transcriptList?.restoreItemAnchor(anchor.key, anchor.offset);
    } finally { loading = false; }
  }
  function followChange(following: boolean) {
    buffer.setFollowing(following);
    if (following && olderMessages.length) olderMessages = [];
  }
  function channelFollowChange(following: boolean) { channelBuffer.setFollowing(following); }
  onMount(() => {
    void refresh();
    (window as unknown as { __syncQA: unknown }).__syncQA = {
      refresh,
      loadEarlier,
      showChannel: () => { channelMode = true; },
      switchChannel: (id: string) => { selectedChannelId = id; },
      currentChannel: () => selectedChannelId,
      channelHistory: (id: string) => snapshot.channels.find(channel => channel.id === id)?.messages.length ?? 0,
      state: () => ({ snapshot, messages, displayMessages: displayed.messages, held: buffer.held(), pending: buffer.pendingUpdates(), cursor: nextBeforeId, loading }),
      channelHeld: () => channelBuffer.held(),
    };
  });
</script>

{#if !channelMode}<section class="fixture-pane">
  <MessagePane active resetKey="ui-sync-test" pendingUpdates={buffer.pendingUpdates()} onfollowchange={followChange}>
    {#snippet header()}
      {#if canLoadEarlier}<div class="pager"><button type="button" aria-label="Load earlier messages" disabled={loading} onclick={() => void loadEarlier()}>{loading ? 'Loading…' : 'Load earlier messages'}</button></div>{/if}
    {/snippet}
    <TranscriptVirtualList bind:this={transcriptList} items={messages} getKey={message => message.id} active>
      {#snippet children(message)}<article class="message" data-message-id={message.id}><b>{message.id}</b><span data-message-text>{message.text}</span></article>{/snippet}
    </TranscriptVirtualList>
  </MessagePane>
</section>{:else}<section class="fixture-pane channel-pane">
  <ChannelTranscriptPane channelId={selectedChannelId} messages={displayedChannel.messages} getChannelMessages={bridge.getChannelMessages} resetKey="channel-sync-test" pendingUpdates={channelBuffer.pendingUpdates()} onfollowchange={channelFollowChange} active>
    {#snippet children(message, _index, previous)}
      {#if !previous || Math.floor(previous.createdAt / 10) !== Math.floor(message.createdAt / 10)}<time class="channel-day">day {Math.floor(message.createdAt / 10)}</time>{/if}
      <article class="message" data-channel-message-id={message.id}><b>{message.id}</b><span data-message-text>{message.text}</span></article>
    {/snippet}
    {#snippet footer()}{#if !displayedChannel.messages.length}<p>Empty channel</p>{/if}{/snippet}
  </ChannelTranscriptPane>
</section>{/if}

<style>
  .fixture-pane { display:flex; width:760px; height:560px; }
  .pager { display:flex; justify-content:center; padding:6px; }
  .message { min-height:52px; box-sizing:border-box; display:flex; gap:12px; border-bottom:1px solid #ccc; padding:10px; }
  .channel-day { display:block; padding:4px 0; color:#667; }
</style>
