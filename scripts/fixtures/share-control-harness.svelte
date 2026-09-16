<script lang="ts">
  import ShareControl from '../../src/lib/components/ShareControl.svelte';
  let open = $state(true);
</script>

<main>
  <ShareControl bind:open taskId="11111111-1111-4111-8111-111111111111" />
</main>

<style>
  :global(html,body,#app){margin:0;min-height:100%;} :global(:root){--panel:#fff;--paper:#f7f8f8;--soft:#edf0f0;--line:#ccd4d4;--ink:#162020;--muted:#5d6868;--accent:#3978d4;--accent-ink:#225aa8;--on-accent:#fff;--mono:ui-monospace,monospace;}
</style>
