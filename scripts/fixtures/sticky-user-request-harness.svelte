<script lang="ts">
  import MessagePane from '../../src/lib/components/MessagePane.svelte';

  const request = 'Please review the complete shipment-tracking workflow and explain every state transition, exception, notification, and recovery path. '.repeat(6).trim();
  const response = 'A long assistant response keeps the original request well above the visible reading position. '.repeat(160);
</script>

<main>
  <MessagePane resetKey="sticky-request" contextAnchorId="sticky-user-message" contextText={request}>
    <article class="user" data-context-message-id="sticky-user-message">{request}</article>
    <article>{response}</article>
  </MessagePane>
</main>

<style>
  :global(html,body,#app) { height:100%; margin:0; overflow:hidden; }
  main { display:flex; height:100%; --paper:#f4f6f7; --panel:#fff; --soft:#e7ebed; --line:#cad1d4; --ink:#182024; --muted:#68747a; --accent:#0ca7dd; --accent-ink:#007cac; --mono:"IBM Plex Mono",monospace; --chat-font:"IBM Plex Sans",sans-serif; --chat-font-size:16px; --chat-line-height:1.55; }
  article { margin:0 0 24px; color:var(--ink); font:16px/1.55 var(--chat-font); }
  article.user { margin-left:auto; max-width:80%; padding:12px 14px; border-radius:10px 10px 3px 10px; background:var(--soft); }
</style>
