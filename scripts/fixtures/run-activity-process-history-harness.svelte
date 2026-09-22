<script lang="ts">
  import { onMount } from 'svelte';
  import RunActivity from '../../src/lib/components/RunActivity.svelte';
  import type { Message, RunEvent } from '../../src/lib/types';
  import { groupConversationActivity } from '../../src/lib/activity-grouping';

  const taskId = 'task-process-history';
  let messages = $state<Message[]>([]);
  let events = $state<RunEvent[]>([]);
  let running = $state(false);

  const user = (id: string, text: string, createdAt: number): Message => ({
    id, taskId, role: 'user', text, createdAt, streamStatus: 'complete',
  });
  const assistant = (id: string, text: string, createdAt: number): Message => ({
    id, taskId, role: 'assistant', text, createdAt, streamStatus: 'complete', phase: 'final_answer',
  });
  const reasoning = (id: string, detail: string, createdAt: number): RunEvent => ({
    id, taskId, kind: 'reasoning', title: 'Reasoning', detail, createdAt,
  });
  const command = (id: string, commandText: string, status: string, createdAt: number): RunEvent => ({
    id, taskId, kind: 'tool', title: 'command_execution',
    detail: JSON.stringify({ type: 'command_execution', command: commandText, status, output: status === 'completed' ? `output for ${commandText}` : '' }),
    createdAt,
  });

  const conversation = $derived(groupConversationActivity(messages, events));

  function reset() {
    messages = [];
    events = [];
    running = false;
  }

  onMount(() => {
    (window as Window & { __PROCESS_HISTORY_QA__?: Record<string, () => void> }).__PROCESS_HISTORY_QA__ = {
      startFirstTurn: () => {
        reset();
        messages = [user('user-1', 'Inspect the first command.', 1_000)];
        events = [reasoning('reasoning-blank-1', '{"type":"reasoning","summary":[]}', 1_100)];
        running = true;
      },
      startFirstCommand: () => {
        events = [...events, command('command-1-start', 'printf first', 'started', 1_200)];
      },
      completeFirstCommand: () => {
        events = [...events, command('command-1-complete', 'printf first', 'completed', 1_300)];
      },
      firstReplyAndSecondTurn: () => {
        messages = [
          ...messages,
          assistant('reply-1', 'First command finished.', 1_400),
          user('user-2', 'Run the second command.', 2_000),
        ];
        events = [...events, command('command-2-start', 'printf second', 'started', 2_100), command('command-2-complete', 'printf second', 'completed', 2_200)];
        running = false;
      },
      summaryScenario: () => {
        reset();
        messages = [user('user-summary', 'Explain the command.', 3_000)];
        events = [
          reasoning('reasoning-summary-1', '{"type":"reasoning","summary":[{"type":"summary_text","text":"I checked the command output before replying."}]}', 3_100),
          command('command-summary-start', 'printf summary', 'started', 3_200),
          command('command-summary-complete', 'printf summary', 'completed', 3_300),
        ];
        running = false;
      },
    };
    return () => { delete (window as Window & { __PROCESS_HISTORY_QA__?: unknown }).__PROCESS_HISTORY_QA__; };
  });
</script>

<main aria-label="Process history fixture">
  {#each conversation as item (item.type === 'tool-group' || item.type === 'reasoning-group' || item.type === 'process-group' ? `${item.type}:${item.values[0].id}` : item.value.id)}
    {#if item.type === 'activity'}
      <RunActivity event={item.value}/>
    {:else if item.type === 'reasoning-group'}
      <RunActivity events={item.values} running={running}/>
    {:else if item.type === 'tool-group'}
      <!-- TaskTranscript renders durable tool history through the process tree too. -->
      <RunActivity events={item.values} processTree running={running}/>
    {:else if item.type === 'process-group'}
      <RunActivity events={item.values} processGroups={item.groups} processTree running={running}/>
    {:else if item.type === 'message'}
      <article class={`message ${item.value.role}`} data-message-id={item.value.id}>{item.value.text}</article>
    {/if}
  {/each}
</main>

<style>
  :global(html, body, #app) { margin: 0; }
  main { padding: 20px; --panel: #fff; --line: #ddd; --muted: #667; --ink: #171717; --accent-ink: #056; --interface-font-ratio: 1; --mono: ui-monospace, monospace; }
  .message { margin: 8px 0; padding: 8px; border: 1px solid var(--line); }
</style>
