<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import MessagePane from '../../src/lib/components/MessagePane.svelte';
  import TranscriptVirtualList from '../../src/lib/components/TranscriptVirtualList.svelte';
  import Markdown from '../../src/lib/components/Markdown.svelte';
  import { createTranscriptBuffer } from '../../src/lib/transcript-buffer.svelte';

  type Message = { id: string; role: 'user' | 'assistant'; text: string; createdAt: number; streamStatus: 'streaming' | 'complete' };
  type Geometry = { pane: string; top: number; height: number; gap: number; following: boolean; held: boolean };

  let { pane, chatId, messages, active = true, resetRevision, running, ongeometry }: {
    pane: string;
    chatId: string;
    messages: Message[];
    active?: boolean;
    resetRevision: number;
    running: boolean;
    ongeometry: (value: Geometry) => void;
  } = $props();

  const buffer = createTranscriptBuffer<Message[]>(
    () => chatId,
    () => messages,
    // This fixture mutates only the final streaming message. Keep the held
    // reader probe cheap: unlike a JSON serialization it does not traverse
    // all 1,000 entries on every 100ms fixture update.
    () => {
      const latest = messages.at(-1);
      return `${messages.length}:${latest?.id ?? ''}:${latest?.text.length ?? 0}:${latest?.streamStatus ?? ''}`;
    },
  );
  const displayed = $derived(buffer.value());
  let following = $state(true);
  let root = $state<HTMLElement>();

  function sampleGeometry() {
    const viewport = root?.querySelector<HTMLElement>('.messages');
    if (!viewport) return;
    ongeometry({
      pane,
      top: Math.round(viewport.scrollTop),
      height: Math.round(viewport.clientHeight),
      gap: Math.round(Math.max(0, viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop)),
      following,
      held: buffer.held(),
    });
  }

  $effect(() => {
    if (!running) return;
    // Reporting geometry updates parent state. Keep that callback outside this
    // effect's dependency graph so parent rendering cannot re-run this effect.
    untrack(sampleGeometry);
    const timer = window.setInterval(() => untrack(sampleGeometry), 100);
    return () => window.clearInterval(timer);
  });

  onMount(() => {
    untrack(sampleGeometry);
  });
</script>

<section bind:this={root} class="fixture-pane" aria-label={`${pane} synthetic transcript`}>
  <MessagePane active={active} resetKey={`${chatId}:${resetRevision}`} pendingUpdates={buffer.pendingUpdates()} onfollowchange={(value) => { following = value; buffer.setFollowing(value); sampleGeometry(); }}>
    <TranscriptVirtualList items={displayed} getKey={(message) => message.id} active={active}>
      {#snippet children(message)}
        <article class:assistant={message.role === 'assistant'} class="message" data-live-entry={message.streamStatus === 'streaming'}>
          <header><span>{message.role === 'assistant' ? 'Synthetic agent' : 'Synthetic operator'}</span><time>{new Date(message.createdAt).toLocaleTimeString()}</time></header>
          <Markdown text={message.text}/>
        </article>
      {/snippet}
    </TranscriptVirtualList>
  </MessagePane>
</section>

<style>
  .fixture-pane { min-width: 0; min-height: 0; display: flex; flex: 1; border: 1px solid var(--line); border-radius: 8px; overflow: hidden; background: var(--paper); }
  .fixture-pane :global(.message) { margin: 0 0 14px; padding: 10px 12px; border: 1px solid var(--line); border-radius: 8px; background: var(--panel); }
  .fixture-pane :global(.message.assistant) { background: var(--soft); }
  .fixture-pane :global(.message header) { display: flex; justify-content: space-between; gap: 10px; margin-bottom: 5px; color: var(--muted); font: 600 11px var(--interface-font, system-ui); }
  .fixture-pane :global(.message time) { font: 10px var(--mono, ui-monospace, monospace); }
</style>
