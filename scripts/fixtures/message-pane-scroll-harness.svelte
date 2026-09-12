<script lang="ts">
  import { onMount, tick } from 'svelte';
  import MessagePane from '../../src/lib/components/MessagePane.svelte';

  type Entry = { id: number; text: string; tall: boolean };
  const paragraph = 'A deliberately long Monitter conversation entry stays inside the message viewport while its layout settles. ';
  let resetKey = $state('initial');
  let entries = $state<Entry[]>(Array.from({ length: 18 }, (_, id) => ({ id, text: `Initial message ${id}. ${paragraph.repeat(10)}`, tall: false })));
  let nextId = 18;

  function append(label: string, tall = false) {
    entries = [...entries, { id: nextId++, text: `${label}. ${paragraph.repeat(10)}`, tall }];
  }

  async function growThenScrollBeforeObserver() {
    append('Late image layout', true);
    await tick();
    document.querySelector('.messages')?.dispatchEvent(new Event('scroll'));
  }

  function growExisting() {
    entries = entries.map((entry, index) => index === entries.length - 1
      ? { ...entry, text: `${entry.text} ${paragraph.repeat(18)}` }
      : entry);
  }

  async function shrinkThenGrow() {
    entries = entries.map((entry, index) => index === entries.length - 1 ? { ...entry, text: 'Shrunk reply.', tall: false } : entry);
    await tick();
    await new Promise(requestAnimationFrame);
    append('Reply grew after shrink', true);
  }

  onMount(() => {
    (window as Window & { __PANE_QA__?: unknown }).__PANE_QA__ = {
      append: () => append('Streaming reply growth'),
      growExisting,
      send: () => { append('Sent message'); resetKey = `send:${nextId}`; },
      growThenScrollBeforeObserver,
      shrinkThenGrow,
    };
    return () => { delete (window as Window & { __PANE_QA__?: unknown }).__PANE_QA__; };
  });
</script>

<main>
  <MessagePane {resetKey}>
    {#each entries as entry (entry.id)}
      <article class:tall={entry.tall}>{entry.text}</article>
    {/each}
  </MessagePane>
</main>

<style>
  :global(html, body, #app) { height: 100%; margin: 0; overflow: hidden; }
  main { height: 100%; display: flex; background: #fff; --paper: #fff; --panel: #fff; --ink: #171717; --line: #ddd; --accent: #066; --accent-ink: #044; --soft: #eef8f8; }
  article { margin: 0 0 16px; padding: 12px; border: 1px solid #ddd; border-radius: 8px; }
  article.tall { min-height: 360px; }
</style>
