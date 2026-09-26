<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import type { Agent, Message, Snapshot, Task } from '$lib/types';
  import { getBridge } from '$lib/bridge';
  import { mergeOlderTranscriptMessages } from '$lib/ui-sync';
  import { createTranscriptBuffer } from '$lib/transcript-buffer.svelte';
  import MessagePane from '$lib/components/MessagePane.svelte';
  import TranscriptVirtualList from '$lib/components/TranscriptVirtualList.svelte';

  const taskId = '11111111-1111-4111-8111-111111111111';
  const bridge = getBridge();
  let snapshot = $state<Snapshot>({ hosts: [], agents: [], tasks: [], messages: [], events: [], channels: [], projects: [], collaborations: [], queuedMessages: [], approvalRequests: [], approvalRules: [], settings: { accent: '#3978d4', theme: 'light', interfaceScale: 100, showToolActivity: true, showReasoningSummaries: true, sendWithEnter: false, sidebarView: 'standard' } });
  let olderMessages = $state<Message[]>([]);
  let nextBeforeId = $state<string | null | undefined>();
  let loading = $state(false);
  const buffer = createTranscriptBuffer(() => taskId, () => ({ messages: snapshot.messages }), () => JSON.stringify(snapshot.messages.map(message => [message.id, message.text])));
  const displayed = $derived(buffer.value());
  const messages = $derived(mergeOlderTranscriptMessages(displayed.messages.filter(message => message.taskId === taskId), olderMessages));
  const canLoadEarlier = $derived(nextBeforeId === undefined ? displayed.messages.filter(message => message.taskId === taskId).length >= 64 : nextBeforeId !== null);

  async function refresh() { snapshot = await bridge.getSnapshot(); }
  async function loadEarlier() {
    if (!bridge.getTaskMessages || loading || !canLoadEarlier) return;
    const visible = messages;
    const beforeId = nextBeforeId ?? visible[0]?.id;
    if (!beforeId) { nextBeforeId = null; return; }
    loading = true;
    try {
      const page = await bridge.getTaskMessages(taskId, beforeId, 16);
      olderMessages = mergeOlderTranscriptMessages(olderMessages, page.messages);
      nextBeforeId = page.nextBeforeId === beforeId ? null : page.nextBeforeId;
    } finally { loading = false; }
  }
  function followChange(following: boolean) { buffer.setFollowing(following); }
  onMount(() => {
    void refresh();
    (window as unknown as { __syncQA: unknown }).__syncQA = {
      refresh,
      loadEarlier,
      state: () => ({ snapshot, messages, held: buffer.held(), pending: buffer.pendingUpdates(), cursor: nextBeforeId }),
    };
  });
</script>

<section class="fixture-pane">
  <MessagePane active resetKey="ui-sync-test" pendingUpdates={buffer.pendingUpdates()} onfollowchange={followChange}>
    {#snippet header()}
      {#if canLoadEarlier}<div class="pager"><button type="button" aria-label="Load earlier messages" disabled={loading} onclick={() => void loadEarlier()}>{loading ? 'Loading…' : 'Load earlier messages'}</button></div>{/if}
    {/snippet}
    <TranscriptVirtualList items={messages} getKey={message => message.id} active>
      {#snippet children(message)}<article class="message" data-message-id={message.id}><b>{message.id}</b><span data-message-text>{message.text}</span></article>{/snippet}
    </TranscriptVirtualList>
  </MessagePane>
</section>

<style>
  .fixture-pane { display:flex; width:760px; height:560px; }
  .pager { display:flex; justify-content:center; padding:6px; }
  .message { min-height:52px; box-sizing:border-box; display:flex; gap:12px; border-bottom:1px solid #ccc; padding:10px; }
</style>
