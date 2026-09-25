<script lang="ts">
  import LazyRichMarkdownComposer from '../../src/lib/components/LazyRichMarkdownComposer.svelte';

  let shown = $state(false);
  let value = $state('initial markdown');
  let inputCalls = $state<string[]>([]);
  let keyCalls = $state<string[]>([]);

  function handleInput(next: string) { inputCalls.push(next); }
  function handleKeydown(event: KeyboardEvent) { keyCalls.push(event.key); }

</script>

<button type="button" onclick={() => shown = !shown}>{shown ? 'Unmount editor' : 'Mount editor'}</button>
<output data-testid="bound-value">{value}</output>
<output data-testid="input-calls">{JSON.stringify(inputCalls)}</output>
<output data-testid="key-calls">{JSON.stringify(keyCalls)}</output>
{#if shown}
  <LazyRichMarkdownComposer bind:value placeholder="Write markdown" oninput={handleInput} onkeydown={handleKeydown} />
{/if}
