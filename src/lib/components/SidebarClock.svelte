<script lang="ts">
  import { ChevronDown } from '@lucide/svelte';
  import { onMount } from 'svelte';
  import { getBridge } from '$lib/bridge';
  import type { ProcessMetricsSample } from '$lib/types';

  let { expanded = $bindable(true) } = $props<{ expanded?: boolean }>();

  const ticks = Array.from({ length: 12 });
  const storageKey = 'monitter.sidebar-clock-expanded.v1';
  const bridge = getBridge();
  let now = $state(new Date());
  let metrics = $state<ProcessMetricsSample | null>(null);
  let previousMetrics: ProcessMetricsSample | null = null;
  let cpuPercent = $state<number | null>(null);
  let metricsError = $state('');
  let metricsLoading = false;

  const hourAngle = $derived(((now.getHours() % 12) + now.getMinutes() / 60) * 30);
  const minuteAngle = $derived((now.getMinutes() + now.getSeconds() / 60) * 6);
  const secondAngle = $derived(now.getSeconds() * 6);
  const shortTime = $derived(new Intl.DateTimeFormat(undefined, {
    hour: '2-digit',
    minute: '2-digit',
  }).format(now));
  const fullTime = $derived(new Intl.DateTimeFormat(undefined, {
    hour: 'numeric',
    minute: '2-digit',
    second: '2-digit',
  }).format(now));
  const cpuText = $derived(cpuPercent === null ? '···' : `${cpuPercent.toFixed(cpuPercent >= 100 ? 0 : 1)}%`);
  const memoryText = $derived(metrics ? formatMemory(metrics.residentMemoryBytes) : '···');

  function formatMemory(bytes: number) {
    const mib = bytes / (1024 * 1024);
    return mib >= 1024 ? `${(mib / 1024).toFixed(1)} GB` : `${Math.round(mib)} MB`;
  }

  async function updateMetrics() {
    if (!expanded || metricsLoading) return;
    metricsLoading = true;
    try {
      const next = await bridge.getProcessMetrics();
      if (previousMetrics) {
        const elapsed = next.sampledAt - previousMetrics.sampledAt;
        const cpuElapsed = next.cpuTimeMs - previousMetrics.cpuTimeMs;
        if (elapsed > 0 && cpuElapsed >= 0) cpuPercent = Math.max(0, cpuElapsed / elapsed * 100);
      }
      previousMetrics = next;
      metrics = next;
      metricsError = '';
    } catch (reason) {
      metricsError = String(reason || 'Process metrics are unavailable.');
    } finally {
      metricsLoading = false;
    }
  }

  onMount(() => {
    try {
      const stored = localStorage.getItem(storageKey);
      if (stored === 'true' || stored === 'false') expanded = stored === 'true';
    } catch { /* Keep the expanded default when local storage is unavailable. */ }

    void updateMetrics();
    const clockTimer = window.setInterval(() => { now = new Date(); }, 1_000);
    const metricsTimer = window.setInterval(() => { void updateMetrics(); }, 2_000);
    return () => {
      window.clearInterval(clockTimer);
      window.clearInterval(metricsTimer);
    };
  });

  function toggle() {
    expanded = !expanded;
    try { localStorage.setItem(storageKey, String(expanded)); } catch { /* The live state still works. */ }
    if (expanded) void updateMetrics();
  }
</script>

<section class="sidebar-clock-widget" class:expanded aria-label="Clock and Monitter usage widget">
  <button class="widget-toggle" type="button" aria-expanded={expanded} aria-controls="sidebar-clock-body" onclick={toggle}>
    <span>LOCAL TIME</span>
    <time datetime={now.toISOString()}>{shortTime}</time>
    <ChevronDown class="widget-chevron" size={13} aria-hidden="true" />
  </button>
  {#if expanded}
    <div class="widget-body" id="sidebar-clock-body">
      <svg class="clock-face" viewBox="0 0 60 60" width="60" height="60" role="img" aria-label={`Analog clock showing ${fullTime}`}>
        <circle class="clock-rim" cx="30" cy="30" r="27.5" />
        {#each ticks as _, index}
          <line class:quarter={index % 3 === 0} x1="30" y1="5" x2="30" y2={index % 3 === 0 ? 10 : 8} transform={`rotate(${index * 30} 30 30)`} />
        {/each}
        <line class="hour-hand" x1="30" y1="31" x2="30" y2="18" transform={`rotate(${hourAngle} 30 30)`} />
        <line class="minute-hand" x1="30" y1="32" x2="30" y2="11" transform={`rotate(${minuteAngle} 30 30)`} />
        <line class="second-hand" x1="30" y1="34" x2="30" y2="9" transform={`rotate(${secondAngle} 30 30)`} />
        <circle class="clock-pin" cx="30" cy="30" r="2" />
      </svg>
      <div class="process-metrics" aria-label="Monitter process usage" title={metricsError || 'Current Monitter host process usage'}>
        {#if metricsError}
          <span class="metrics-error">METRICS<br />UNAVAILABLE</span>
        {:else}
          <div><span>CPU</span><strong>{cpuText}</strong></div>
          <div><span>RAM</span><strong>{memoryText}</strong></div>
        {/if}
      </div>
    </div>
  {/if}
</section>

<style>
  .sidebar-clock-widget {
    position: absolute;
    right: 0;
    bottom: calc(var(--density-sidebar-footer-height) + var(--sidebar-footer-safe-area, 0px));
    left: 0;
    z-index: 4;
    overflow: hidden;
    border-top: 1px solid var(--line);
    background: color-mix(in srgb, var(--sidebar) 94%, transparent);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
  }
  .widget-toggle {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto 16px;
    align-items: center;
    width: 100%;
    height: 31px;
    padding: 0 10px 0 13px;
    color: var(--muted);
    text-align: left;
  }
  .widget-toggle:hover { color: var(--ink); background: var(--soft); }
  .widget-toggle > span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: calc(9px * var(--interface-font-ratio, 1));
    font-weight: 600;
    letter-spacing: .09em;
  }
  .widget-toggle time { font: calc(10px * var(--interface-font-ratio, 1)) var(--mono); }
  .widget-toggle :global(.widget-chevron) { justify-self: end; transform: rotate(180deg); transition: transform 150ms ease-out; }
  .expanded .widget-toggle :global(.widget-chevron) { transform: rotate(0); }
  .widget-body {
    display: flex;
    align-items: center;
    gap: 13px;
    height: 77px;
    padding: 7px 13px 10px;
    border-top: 1px solid color-mix(in srgb, var(--line) 65%, transparent);
  }
  .clock-face { flex: none; color: var(--ink); }
  .clock-rim { fill: color-mix(in srgb, var(--panel) 72%, transparent); stroke: var(--line); stroke-width: 1; }
  .clock-face line { stroke: var(--muted); stroke-linecap: round; stroke-width: 1; }
  .clock-face line.quarter { stroke: var(--ink); stroke-width: 1.5; }
  .clock-face .hour-hand { stroke: var(--ink); stroke-width: 2.7; }
  .clock-face .minute-hand { stroke: var(--ink); stroke-width: 1.8; }
  .clock-face .second-hand { stroke: var(--accent); stroke-width: 1; }
  .clock-pin { fill: var(--accent); stroke: var(--panel); stroke-width: 1; }
  .process-metrics { display:grid; flex:1; min-width:0; align-self:stretch; align-content:center; gap:7px; }
  .process-metrics > div { display:flex; align-items:baseline; justify-content:space-between; gap:7px; min-width:0; }
  .process-metrics span { color:var(--muted); font-size:calc(9px * var(--interface-font-ratio,1)); font-weight:600; letter-spacing:.08em; }
  .process-metrics strong { overflow:hidden; color:var(--ink); text-overflow:ellipsis; white-space:nowrap; font:500 calc(11px * var(--interface-font-ratio,1)) var(--mono); }
  .process-metrics .metrics-error { color:var(--muted); line-height:1.45; }
  @media (prefers-reduced-motion: reduce) { .widget-toggle :global(.widget-chevron) { transition: none; } }
</style>
