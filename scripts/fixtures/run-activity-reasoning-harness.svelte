<script lang="ts">
  import { onMount } from 'svelte';
  import RunActivity from '../../src/lib/components/RunActivity.svelte';
  import ThinkingStatus from '../../src/lib/components/ThinkingStatus.svelte';
  import type { Message, RunEvent } from '../../src/lib/types';
  import { groupConversationActivity, showThinkingFallback } from '../../src/lib/activity-grouping';

  let running = $state(false);
  let detail = $state('{"content":[],"summary":[],"type":"reasoning"}');
  let messages = $state<Message[]>([]);
  let includeEvent = $state(true);
  let compactionRunning = $state(true);
  let compactionEvents = $state<RunEvent[]>([
    { id: 'compact-start', taskId: 'task-1', kind: 'tool', title: 'ContextCompaction', detail: '{"type":"ContextCompaction","id":"compact-1","monitterPhase":"started"}', createdAt: 1 },
  ]);
  const reasoningStartedAt = Date.now() - 201_000;
  const event = $derived<RunEvent>({
    id: 'reasoning-1', taskId: 'task-1', kind: 'reasoning', title: 'Reasoning', detail, createdAt: reasoningStartedAt,
  });
  const imageEvent: RunEvent = {
    id: 'view-image-1', taskId: 'task-1', kind: 'tool', title: 'imageView',
    detail: '{"type":"imageView","path":"/tmp/cargo.png"}', createdAt: reasoningStartedAt + 1,
  };
  const conversation = $derived(groupConversationActivity(messages, includeEvent ? [event] : []));

  onMount(() => {
    window.__MONITTER_TEST_BRIDGE__ = {
      invoke: async command => {
        if (command === 'read_attachment_file') return {
          filename: 'cargo.png', mimeType: 'image/png',
          dataBase64: 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aX1sAAAAASUVORK5CYII=',
        };
        throw new Error(`Unexpected test bridge command: ${command}`);
      },
      listen: async () => () => {},
    };
    (window as Window & { __REASONING_QA__?: Record<string, () => void> }).__REASONING_QA__ = {
      activate: () => { running = true; },
      deactivate: () => { running = false; },
      summary: () => { detail = '{"type":"reasoning","summary":[{"type":"summary_text","text":"I checked the source and found the relevant path."}]}'; },
      emptyReply: () => { messages = [{ id: 'reply', taskId: 'task-1', role: 'assistant', text: '', createdAt: Date.now(), streamStatus: 'streaming' }]; },
      reply: () => { messages = [{ id: 'reply', taskId: 'task-1', role: 'assistant', text: 'Here is the reply.', createdAt: Date.now(), streamStatus: 'streaming' }]; },
      resetReply: () => { messages = []; },
      beginWaiting: () => { messages = []; includeEvent = false; running = true; detail = ''; },
      reportReasoning: () => { includeEvent = true; },
      completeCompaction: () => { compactionEvents = [...compactionEvents, { id: 'compact-complete', taskId: 'task-1', kind: 'tool', title: 'ContextCompaction', detail: '{"type":"ContextCompaction","id":"compact-1","monitterPhase":"completed"}', createdAt: 15_301 }]; },
      interruptCompaction: () => { compactionRunning = false; compactionEvents = [compactionEvents[0]]; },
    };
    return () => { delete window.__MONITTER_TEST_BRIDGE__; delete (window as Window & { __REASONING_QA__?: unknown }).__REASONING_QA__; };
  });
</script>

{#snippet avatar()}<span class="message-avatar" role="img" aria-label="Agent avatar">A</span>{/snippet}
<main>
  {#each conversation as item (item.type === 'reasoning-group' || item.type === 'tool-group' ? item.values[0].id : item.value.id)}
    {#if item.type === 'reasoning-group'}<RunActivity events={item.values} {running} {avatar}/>
    {:else if item.type === 'activity'}<RunActivity event={item.value} {running}/>
    {:else if item.type === 'message' && item.value.text}<p data-testid="reply">{item.value.text}</p>{/if}
  {/each}
  {#if showThinkingFallback(conversation, running)}<ThinkingStatus {running} {avatar}/>{/if}
  <RunActivity events={compactionEvents} running={compactionRunning}/>
  <RunActivity event={imageEvent}/>
</main>

<style>
  :global(html, body, #app) { margin: 0; }
  main { padding: 20px; --panel: #fff; --line: #ddd; --muted: #667; --ink: #171717; --accent-ink: #056; --interface-font-ratio: 1; --mono: ui-monospace, monospace; }
  .message-avatar { display:grid; place-items:center; width:20px; height:20px; background:#056; color:white; border-radius:4px; }
</style>
