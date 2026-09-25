<script lang="ts">
  let label = 'first';
  let selected = 'one';
</script>

{#snippet row(item: string, depth = 0)}
  <div class="row" data-depth={depth}>
    <button class="row-item">{item}</button>
    {#if depth === 0}{@render row(`${item} nested`, depth + 1)}{/if}
  </div>
{/snippet}

{#snippet group(item: string)}
  <section class="group">
    {@render row(item)}
    {@render row(`${item} second`)}
  </section>
{/snippet}

<main class="fixture-root">
  {@render group('alpha')}
  {@render group(label)}
  <select bind:value={selected}>
    <selectedcontent><span class="selected-copy">{selected}</span></selectedcontent>
    <option value="one"><span class="option-copy">One</span></option>
    <option value="two"><span class="option-copy">Two</span></option>
  </select>
</main>

<style>
  .fixture-root > .group .row > .row-item { color: red; }
  .fixture-root :is(.group, .other) .row-item:hover { color: blue; }
  .fixture-root:has(.row-item) > select > selectedcontent > .selected-copy { font-weight: 600; }
  select > option .option-copy { display: inline; }
</style>
