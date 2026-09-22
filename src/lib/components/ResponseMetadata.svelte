<script lang="ts">
  import { Info } from '@lucide/svelte';
  import type { AssistantResponseMetadata } from '$lib/types';

  let { metadata }: { metadata: AssistantResponseMetadata } = $props();

  const integer = new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 });
  const totalTokens = $derived(metadata.inputTokens + metadata.outputTokens);
  const routeSummary = $derived(metadata.routeApplied === true
    ? 'Jev applied this route.'
    : metadata.applicationError
      ? 'Jev proposed a route, but Mona kept the working runtime.'
      : metadata.routeApplied === false
        ? 'Jev kept the current runtime.'
        : 'Jev routing details');
</script>

<details class="response-metadata">
  <summary aria-label={`Response details for ${metadata.model}`}>
    <span>{metadata.model}</span>
    <Info size={12} strokeWidth={1.8} aria-hidden="true" />
  </summary>
  <div class="response-details">
    <dl>
      <div><dt>Tokens</dt><dd>{integer.format(totalTokens)} total</dd></div>
      <div><dt>Input</dt><dd>{integer.format(metadata.inputTokens)}</dd></div>
      <div><dt>Output</dt><dd>{integer.format(metadata.outputTokens)}</dd></div>
      {#if metadata.requestedModel}<div><dt>Jev requested</dt><dd>{metadata.requestedModel}{metadata.requestedEffort ? ` · ${metadata.requestedEffort}` : ''}</dd></div>{/if}
      {#if metadata.confidence != null}<div><dt>Confidence</dt><dd>{Math.round(metadata.confidence * 100)}%</dd></div>{/if}
    </dl>
    <p class="route-summary">{routeSummary}</p>
    {#if metadata.jevRationale}<p>{metadata.jevRationale}</p>{/if}
    {#if metadata.applicationError}<p class="route-error">{metadata.applicationError}</p>{/if}
  </div>
</details>

<style>
  .response-metadata {
    position: relative;
    width: fit-content;
    margin: 6px 0 0 1px;
    color: var(--muted);
    font: 500 calc(10px * var(--interface-font-ratio, 1))/1.3 var(--mono, "IBM Plex Mono", ui-monospace, monospace);
  }
  summary {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: min(360px, 75vw);
    cursor: pointer;
    list-style: none;
    opacity: .78;
  }
  summary::-webkit-details-marker { display: none; }
  summary:hover, details[open] summary { color: var(--ink); opacity: 1; }
  summary span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  summary :global(svg) { flex: none; }
  .response-details {
    z-index: 8;
    width: min(340px, calc(100vw - 48px));
    margin-top: 6px;
    padding: 10px 11px;
    border: 1px solid var(--line);
    border-radius: 8px;
    color: var(--ink);
    background: var(--panel);
    box-shadow: 0 10px 28px rgba(0, 0, 0, .18);
    font-family: var(--interface-font, "IBM Plex Sans", system-ui, sans-serif);
    font-size: calc(11px * var(--interface-font-ratio, 1));
    line-height: 1.4;
  }
  dl { display: grid; gap: 4px; margin: 0; }
  dl div { display: flex; justify-content: space-between; gap: 16px; }
  dt { color: var(--muted); }
  dd { margin: 0; font-family: var(--mono, "IBM Plex Mono", ui-monospace, monospace); text-align: right; overflow-wrap: anywhere; }
  p { margin: 8px 0 0; overflow-wrap: anywhere; }
  .route-summary { color: var(--muted); }
  .route-error { color: var(--danger, #d66); }
</style>
