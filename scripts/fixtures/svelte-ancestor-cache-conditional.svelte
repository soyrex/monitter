<script lang="ts">
  let enabled = true;
</script>

{#snippet title(text: string)}<span class="title">{text}</span>{/snippet}
{#snippet panel(text: string)}<article class="panel">{@render title(text)}{@render title(`${text} detail`)}</article>{/snippet}

{#if enabled}
  {@render panel('Primary')}
{:else}
  {@render panel('Fallback')}
{/if}
<aside class="secondary">{@render title('Secondary')}</aside>

<style>
  .panel > .title + .title { color: red; }
  .secondary .title { color: blue; }
  .panel:has(.title:is(:hover, :focus)) { outline: 1px solid; }
</style>
