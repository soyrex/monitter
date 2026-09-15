<script lang="ts">
  import type { ComposerContextUsage } from '$lib/context-usage-data';

  let { usage }: { usage: ComposerContextUsage } = $props();

  type Rgb = readonly [red: number, green: number, blue: number];
  const green: Rgb = [63, 157, 106];
  const orange: Rgb = [224, 133, 51];
  const red: Rgb = [205, 76, 76];

  function blend(from: Rgb, to: Rgb, amount: number): string {
    const progress = Math.max(0, Math.min(1, amount));
    return `rgb(${from.map((channel, index) => Math.round(channel + (to[index] - channel) * progress)).join(' ')})`;
  }

  function color(percent: number): string {
    if (percent <= 60) return `rgb(${green.join(' ')})`;
    if (percent <= 90) return blend(green, orange, (percent - 60) / 30);
    return blend(orange, red, (percent - 90) / 10);
  }

  const number = new Intl.NumberFormat();
  const label = $derived(usage.status === 'available'
    ? `Context: ${number.format(usage.used)} of ${number.format(usage.size)} tokens used (${Math.round(usage.usedPercent)}%)${usage.model ? `. ${usage.model}` : ''}`
    : usage.reason);
</script>

{#if usage.status === 'available'}
  <div
    class="context-usage-bar"
    role="meter"
    aria-label={label}
    aria-valuemin="0"
    aria-valuemax="100"
    aria-valuenow={usage.usedPercent}
    aria-valuetext={`${number.format(usage.used)} of ${number.format(usage.size)} tokens used`}
    title={label}
  >
    <span style={`width:${usage.usedPercent}%;background:${color(usage.usedPercent)}`}></span>
  </div>
{:else}
  <div class="context-usage-bar unavailable" role="status" aria-label={label} title={label}></div>
{/if}

<style>
  .context-usage-bar { position:absolute; bottom:0; left:0; right:0; height:3px; overflow:hidden; background:color-mix(in srgb, var(--line) 70%, transparent); }
  .context-usage-bar span { display:block; height:100%; transition:width 160ms ease, background-color 160ms ease; }
  .context-usage-bar.unavailable { opacity:.7; }
  @media (prefers-reduced-motion: reduce) { .context-usage-bar span { transition:none; } }
</style>
