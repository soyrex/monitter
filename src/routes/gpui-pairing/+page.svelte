<script lang="ts">
  import RemoteControl from '$lib/components/RemoteControl.svelte';
  import { pairingHost } from '$lib/controller/gpui-pairing-host';
</script>
<svelte:head><title>Pair a phone — Monitter</title></svelte:head>
<main>
  {#if pairingHost()}
    <RemoteControl open standalone />
  {:else}
    <h1>Pair a phone</h1><p>Open Settings → Pair a phone in the Monitter desktop app.</p>
  {/if}
</main>
<style>
  :global(body){margin:0;background:#faf9f6;color:#242424;--paper:#fff;--ink:#242424;--muted:#666;--line:#deddd8;--soft:#eeede8;--accent:#385b43;--on-accent:#fff;--accent-ink:#385b43;font-family:'IBM Plex Sans',sans-serif}
  main{max-width:680px;margin:0 auto;padding:24px;box-sizing:border-box}
  @media(prefers-color-scheme:dark){:global(body){background:#20211f;color:#eee;--paper:#292b28;--ink:#eee;--muted:#b3b5ae;--line:#45483f;--soft:#34372f;--accent:#8fbc98;--accent-ink:#8fbc98;--on-accent:#18241a}}
</style>
