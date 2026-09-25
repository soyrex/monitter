<script lang="ts">
  import MessagePane from '../../src/lib/components/MessagePane.svelte';
  import TranscriptVirtualList from '../../src/lib/components/TranscriptVirtualList.svelte';

  const rows = Array.from({ length: 36 }, (_, index) => ({
    id: `row-${index}`,
    text: `Conversation entry ${index + 1}. ${'The virtualized transcript should keep its scroll owner connected to the pane. '.repeat(5)}`,
  }));
</script>

<main>
  <MessagePane resetKey="jump-test">
    <TranscriptVirtualList items={rows} getKey={row => row.id} active>
      {#snippet children(row)}<article>{row.text}</article>{/snippet}
      {#snippet footer()}<div class="tail">Latest message</div>{/snippet}
    </TranscriptVirtualList>
  </MessagePane>
</main>

<style>
  :global(html, body, #app) { height:100%; margin:0; overflow:hidden; }
  main { display:flex; width:100%; height:100%; --paper:#fff; --panel:#fff; --ink:#171717; --line:#ddd; --accent:#066; --accent-ink:#044; --soft:#eef8f8; }
  article { margin:0 0 12px; padding:12px; border:1px solid var(--line); border-radius:8px; }
  .tail { padding:12px; color:var(--muted); }
</style>
