<script lang="ts">
  import { Brain, ChevronRight, Terminal } from "@lucide/svelte";
  import type { RunEvent } from "$lib/types";
  import Markdown from "./Markdown.svelte";
  let { event }: { event: RunEvent } = $props();
  const reasoning = $derived(event.kind === "reasoning");
  const time = $derived(new Intl.DateTimeFormat(undefined, {
    hour: "2-digit", minute: "2-digit",
  }).format(event.createdAt));
  function readableDetail(value: string) {
    if (!value || value === "null") return "The harness reported this tool event without additional detail.";
    try { return JSON.stringify(JSON.parse(value), null, 2); }
    catch { return value; }
  }
</script>

<details class="activity" class:reasoning>
  <summary aria-label={reasoning ? "Reasoning summary" : `Tool activity: ${event.title}`}>
    <ChevronRight size={13} class="chevron" />
    {#if reasoning}<Brain size={14} />{:else}<Terminal size={14} />{/if}
    <span>{reasoning ? "Reasoning summary" : event.title || "Tool activity"}</span>
    <time>{time}</time>
  </summary>
  <div class="activity-body">
    {#if reasoning}<Markdown text={event.detail} />
    {:else}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex (scrollable output must be keyboard reachable) -->
      <pre tabindex="0" aria-label={`Tool details: ${event.title}`}>{readableDetail(event.detail)}</pre>
    {/if}
  </div>
</details>

<style>
  .activity { margin: 12px 0; border: 1px solid var(--line); border-radius: 8px; background: var(--panel); font-size: 12px; }
  summary { display: flex; align-items: center; gap: 8px; padding: 11px 12px; color: var(--muted); cursor: pointer; list-style: none; }
  summary::-webkit-details-marker { display: none; }
  summary:focus-visible { outline: 2px solid var(--accent-ink); outline-offset: 2px; }
  summary span { flex: 1; min-width: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  summary time { flex-shrink: 0; font: 10px var(--mono); }
  summary :global(svg) { flex-shrink: 0; }
  .activity[open] :global(.chevron) { transform: rotate(90deg); }
  .reasoning summary :global(svg) { color: var(--accent-ink); }
  .activity-body { padding: 0 14px 12px; overflow: auto; max-height: 280px; }
  pre { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; font: 11px/1.6 var(--mono); }
</style>
