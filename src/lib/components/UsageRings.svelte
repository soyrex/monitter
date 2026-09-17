<script lang="ts">
  import { ChevronDown } from '@lucide/svelte';
  import { onMount, tick } from 'svelte';
  import ProviderIcon from './ProviderIcon.svelte';

  /**
   * A display-only projection of provider allowance data. The owner is
   * responsible for fetching/mapping provider payloads; this component never
   * calls the native bridge and deliberately renders absent values as unknown.
   */
  export type UsageRingProvider = 'codex' | 'claude' | 'minimax' | 'opencode-go';
  export type UsageRingStatus = 'ready' | 'loading' | 'unavailable' | 'error' | 'stale';

  export interface UsageRingWindow {
    /** A short, human-readable allowance window, for example "5h" or "Week". */
    label: string;
    /** A provider-reported percentage. Null means the provider did not report it. */
    usedPercent: number | null;
    /** True when the provider explicitly marks this window as unlimited. */
    unlimited?: boolean;
    /** Unix milliseconds, when the provider supplied a reset timestamp. */
    resetsAt?: number | null;
    /** Optional provider-supplied reset text, preferred over formatting a timestamp. */
    resetLabel?: string | null;
  }

  export interface UsageRingData {
    status?: UsageRingStatus;
    /** The provider's currently active allowance window. */
    active?: UsageRingWindow | null;
    /** A weekly allowance window, if that provider exposes one. */
    weekly?: UsageRingWindow | null;
    /** An actionable, already-redacted message for error/unavailable states. */
    message?: string | null;
    /** Unix milliseconds of the last successful source read, if known. */
    updatedAt?: number | null;
  }

  export type UsageRingMap = Partial<Record<UsageRingProvider, UsageRingData>>;

  type ProviderDefinition = { id: UsageRingProvider; label: string; mark: string };
  const providers: readonly ProviderDefinition[] = [
    { id: 'codex', label: 'Codex', mark: 'C' },
    { id: 'claude', label: 'Claude', mark: 'Cl' },
    { id: 'minimax', label: 'MiniMax', mark: 'M' },
    { id: 'opencode-go', label: 'OpenCode Go', mark: 'Go' },
  ];
  const radius = 15.5;
  const legacyStorageKey = 'monitter.sidebar-usage-expanded.v1';
  const storageKey = 'monitter.sidebar-usage-layout.v2';
  type UsageLayoutPhase = 0 | 1 | 2 | 3;

  let {
    usage = {},
    compact = false,
    expanded = $bindable(true),
    class: className = '',
  }: { usage?: UsageRingMap; compact?: boolean; expanded?: boolean; class?: string } = $props();
  let showAbsoluteResets = $state(false);
  let currentTime = $state(Date.now());
  let ringsOnly = $state(false);
  // A null phase leaves the initial presentation responsive to window height.
  // The two horizontal phases make the toggle's full cycle explicit.
  let layoutPhase = $state<UsageLayoutPhase | null>(null);
  let pane = $state<HTMLElement>();
  let horizontal = $state(false);

  /**
   * Measure the full form before switching layouts. Measuring the compact form
   * itself would immediately drop below the threshold and make it oscillate.
   * The middle state keeps every provider's detail visible while shortening
   * the labels to fit a short sidebar.
   */
  $effect(() => {
    usage; compact; expanded; layoutPhase; pane;
    ringsOnly = !compact && expanded && (layoutPhase === 1 || layoutPhase === 3);
    if (compact || !expanded || !pane || layoutPhase !== null) { horizontal = false; return; }
    let cancelled = false;
    async function measure() {
      horizontal = false;
      await tick();
      if (!cancelled && pane) horizontal = pane.getBoundingClientRect().height > window.innerHeight * 0.3;
    }
    void measure();
    const onResize = () => void measure();
    window.addEventListener('resize', onResize);
    return () => {
      cancelled = true;
      window.removeEventListener('resize', onResize);
    };
  });

  onMount(() => {
    try {
      const stored = localStorage.getItem(storageKey);
      const parsed = stored ? JSON.parse(stored) : null;
      if (parsed?.version === 2 && [0, 1, 2, 3].includes(parsed.phase)) {
        layoutPhase = parsed.phase;
        expanded = layoutPhase !== 0;
      } else {
        const legacy = localStorage.getItem(legacyStorageKey);
        if (legacy === 'true' || legacy === 'false') {
          // Preserve the previous visible state once, then make it deterministic.
          layoutPhase = legacy === 'true' ? (window.outerHeight < 1200 ? 1 : 2) : 0;
          expanded = layoutPhase !== 0;
          persistLayoutPhase();
        }
      }
    } catch { /* Keep the expanded default when local storage is unavailable. */ }

    const refreshCurrentTime = () => { currentTime = Date.now(); };
    const untilNextMinute = 60_000 - (Date.now() % 60_000);
    let minuteTimer: number | undefined;
    const firstRefresh = window.setTimeout(() => {
      refreshCurrentTime();
      minuteTimer = window.setInterval(refreshCurrentTime, 60_000);
    }, untilNextMinute);

    return () => {
      window.clearTimeout(firstRefresh);
      if (minuteTimer !== undefined) window.clearInterval(minuteTimer);
    };
  });

  function persistLayoutPhase(): void {
    if (layoutPhase === null) return;
    try { localStorage.setItem(storageKey, JSON.stringify({ version: 2, phase: layoutPhase })); } catch { /* The live state still works. */ }
  }

  function toggle(): void {
    if (layoutPhase === null) {
      // Advance from the responsive presentation currently visible to the next
      // fixed phase: hidden → horizontal → vertical → horizontal → hidden.
      layoutPhase = !expanded ? 1 : ringsOnly ? 2 : 3;
    } else {
      layoutPhase = ((layoutPhase + 1) % 4) as UsageLayoutPhase;
    }
    expanded = layoutPhase !== 0;
    persistLayoutPhase();
  }

  function toggleAction(): string {
    const current = layoutPhase ?? (!expanded ? 0 : ringsOnly ? 1 : 2);
    const next = ((current + 1) % 4) as UsageLayoutPhase;
    return ['Hide provider usage', 'Show provider usage horizontally', 'Show provider usage vertically', 'Show provider usage horizontally'][next];
  }

  function percent(value: number | null | undefined): number | null {
    return typeof value === 'number' && Number.isFinite(value) ? Math.max(0, Math.min(100, value)) : null;
  }

  function percentText(value: number | null | undefined, unlimited = false): string {
    if (unlimited) return '∞';
    const normalized = percent(value);
    return normalized === null ? '—' : `${Math.round(normalized)}%`;
  }

  type Rgb = readonly [red: number, green: number, blue: number];
  const usageGreen: Rgb = [63, 157, 106];
  const usageOrange: Rgb = [224, 133, 51];
  const usageRed: Rgb = [205, 76, 76];

  function blendColor(from: Rgb, to: Rgb, amount: number): string {
    const progress = Math.max(0, Math.min(1, amount));
    return `rgb(${from.map((channel, index) => Math.round(channel + (to[index] - channel) * progress)).join(' ')})`;
  }

  /**
   * Allowance data is expressed as used percentage, while the warning scale is
   * based on capacity remaining: green above 40%, orange from 40–10%, and red
   * for the final 10%. Interpolating at each boundary keeps the change smooth.
   */
  function usageColor(window: UsageRingWindow | null | undefined): string {
    if (window?.unlimited) return 'var(--accent)';
    const used = percent(window?.usedPercent);
    if (used === null) return 'var(--accent)';
    const remaining = 100 - used;
    if (remaining >= 40) return blendColor(usageGreen, usageGreen, 0);
    if (remaining >= 10) return blendColor(usageOrange, usageGreen, (remaining - 10) / 30);
    return blendColor(usageRed, usageOrange, remaining / 10);
  }

  function resetTimestamp(window: UsageRingWindow | null | undefined): number | null {
    return typeof window?.resetsAt === 'number' && Number.isFinite(window.resetsAt) ? window.resetsAt : null;
  }

  function compactDuration(minutes: number): string {
    const days = Math.floor(minutes / 1_440);
    const hours = Math.floor((minutes % 1_440) / 60);
    const remainingMinutes = minutes % 60;
    return [
      days > 0 ? `${days}d` : '',
      hours > 0 ? `${hours}h` : '',
      remainingMinutes > 0 ? `${remainingMinutes}m` : '',
    ].filter(Boolean).join(' ') || '0m';
  }

  function relativeReset(timestamp: number): { prefix: string; value: string } {
    const difference = timestamp - currentTime;
    if (difference >= 60_000) return { prefix: 'Resets in', value: compactDuration(Math.ceil(difference / 60_000)) };
    if (difference > 0) return { prefix: 'Resets in', value: 'less than a minute' };
    if (difference > -60_000) return { prefix: 'Reset due', value: 'now' };
    return { prefix: 'Reset overdue by', value: compactDuration(Math.floor(Math.abs(difference) / 60_000)) };
  }

  function absoluteReset(timestamp: number): string {
    return new Intl.DateTimeFormat(undefined, {
      year: 'numeric', month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit',
    }).format(timestamp);
  }

  function resetText(window: UsageRingWindow | null | undefined): string {
    if (!window) return 'Reset time unavailable';
    const timestamp = resetTimestamp(window);
    if (timestamp !== null) {
      if (showAbsoluteResets) return `Resets ${absoluteReset(timestamp)}`;
      const relative = relativeReset(timestamp);
      return `${relative.prefix} ${relative.value}`;
    }
    if (window.resetLabel?.trim()) return window.resetLabel.trim();
    return 'Reset time unavailable';
  }

  function toggleResetDisplay(): void {
    showAbsoluteResets = !showAbsoluteResets;
  }

  function resetToggleAction(): string {
    return showAbsoluteResets
      ? 'Show relative reset times for all provider windows'
      : 'Show absolute reset dates and times for all provider windows';
  }

  function resetToggleLabel(window: UsageRingWindow | null | undefined): string {
    return `${resetText(window)}. ${resetToggleAction()}`;
  }

  function stateLabel(data: UsageRingData | undefined): string {
    switch (data?.status ?? 'unavailable') {
      case 'ready': return data?.active ? 'Available' : 'No allowance data';
      case 'loading': return 'Loading';
      case 'stale': return 'Stale';
      case 'error': return 'Could not load';
      default: return 'Unavailable';
    }
  }

  function providerTitle(provider: ProviderDefinition, data: UsageRingData | undefined): string {
    const state = stateLabel(data);
    const active = data?.active;
    const weekly = data?.weekly;
    const windows = [
      active ? `${active.label}: ${percentText(active.usedPercent, active.unlimited)} used. ${resetText(active)}.` : '',
      weekly ? `${weekly.label}: ${percentText(weekly.usedPercent, weekly.unlimited)} used. ${resetText(weekly)}.` : '',
    ].filter(Boolean).join(' ');
    return [provider.label, state, windows, data?.message?.trim() ?? ''].filter(Boolean).join('. ');
  }

  function showValue(data: UsageRingData | undefined): boolean {
    return (data?.status === 'ready' || data?.status === 'stale') && (data.active?.unlimited === true || percent(data.active?.usedPercent) !== null);
  }
</script>

<section bind:this={pane} class:compact class:expanded class:horizontal class:rings-only={ringsOnly} class={`usage-rings ${className}`.trim()} aria-label="Provider usage">
  {#if !compact}
    <button class="usage-toggle" type="button" aria-expanded={expanded} aria-controls="sidebar-usage-body" aria-label={toggleAction()} title={toggleAction()} onclick={toggle}>
      <span>MODEL USAGE</span>
      <small>{providers.length} PROVIDERS</small>
      <ChevronDown class="usage-chevron" size={13} aria-hidden="true" />
    </button>
  {/if}
  {#if expanded || compact}
    <div class="usage-body" id="sidebar-usage-body">
      {#each providers as provider (provider.id)}
        {@const data = usage[provider.id]}
        {@const active = data?.active}
        {@const weekly = data?.weekly}
        {@const hasValue = showValue(data)}
        {@const status = data?.status ?? 'unavailable'}
        <article
          class="usage-provider"
          class:loading={status === 'loading'}
          class:unavailable={status === 'unavailable' || status === 'error' || !hasValue}
          class:stale={status === 'stale'}
          class:error={status === 'error'}
          aria-label={providerTitle(provider, data)}
          title={providerTitle(provider, data)}
        >
          <div class="usage-ring-wrap">
            <svg class="usage-ring" viewBox="0 0 40 40" role="img" aria-label={`${provider.label}: ${hasValue ? `${percentText(active?.usedPercent)} used in ${active?.label}` : stateLabel(data)}`} style={`--ring-active-color:${usageColor(active)};--ring-weekly-color:${usageColor(weekly)}`}>
              <circle class="ring-track" cx="20" cy="20" r={radius} />
              <circle
                class="ring-active"
                cx="20" cy="20" r={radius}
                pathLength="100"
                stroke-dasharray="100"
                stroke-dashoffset={active?.unlimited ? 0 : 100 - (percent(active?.usedPercent) ?? 0)}
              />
              {#if weekly && !weekly.unlimited && (status === 'ready' || status === 'stale')}
                <circle
                  class="ring-weekly"
                  cx="20" cy="20" r="11.5"
                  pathLength="100"
                  stroke-dasharray="100"
                  stroke-dashoffset={100 - (percent(weekly.usedPercent) ?? 0)}
                />
              {/if}
            </svg>
            <span class="ring-value" aria-hidden="true">{hasValue ? percentText(active?.usedPercent, active?.unlimited).replace('%', '') : provider.mark}</span>
          </div>
          <div class="usage-copy">
            <div class="usage-heading">{#if !ringsOnly && !compact}<span class="usage-provider-icon"><ProviderIcon provider={provider.id} size={12} /></span>{/if}<strong>{provider.label}</strong><span class="usage-status">{status === 'ready' ? active?.label ?? stateLabel(data) : stateLabel(data)}</span></div>
            {#if hasValue}
              <div class="usage-detail">
                <span>
                  <span class="usage-detail-long">{active?.unlimited ? 'Unlimited' : `${percentText(active?.usedPercent)} used.`}</span>
                  <span class="usage-detail-short">{active?.unlimited ? 'U: ∞' : `U: ${percentText(active?.usedPercent)}`}</span>
                </span>
                {#if resetTimestamp(active) !== null}
                  {@const activeReset = resetTimestamp(active)!}
                  <button class="usage-reset" type="button" aria-label={resetToggleLabel(active)} title={resetToggleAction()} onclick={toggleResetDisplay}>
                    {#if showAbsoluteResets}
                      <span class="usage-reset-long">Resets {absoluteReset(activeReset)}</span><span class="usage-reset-short">R: {absoluteReset(activeReset)}</span>
                    {:else}
                      {@const relative = relativeReset(activeReset)}
                      <span class="usage-reset-long">{relative.prefix} <strong>{relative.value}</strong></span><span class="usage-reset-short">R: {relative.value}</span>
                    {/if}
                  </button>
                {:else}
                  {#if active?.resetLabel?.trim()}
                    <button class="usage-reset" type="button" aria-label={resetToggleLabel(active)} title={resetToggleAction()} onclick={toggleResetDisplay}><span class="usage-reset-long">{resetText(active)}</span><span class="usage-reset-short">R: {resetText(active)}</span></button>
                  {:else}
                    <span class="usage-reset" title={resetText(active)}><span class="usage-reset-long">{resetText(active)}</span><span class="usage-reset-short">R: {resetText(active)}</span></span>
                  {/if}
                {/if}
              </div>
              {#if weekly}
                <div class="usage-weekly" title={resetText(weekly)}>
                  <span>{weekly.label} {weekly.unlimited ? '∞' : percentText(weekly.usedPercent)}</span>
                  {#if resetTimestamp(weekly) !== null}
                    {@const weeklyReset = resetTimestamp(weekly)!}
                    <button class="usage-reset" type="button" aria-label={resetToggleLabel(weekly)} title={resetToggleAction()} onclick={toggleResetDisplay}>
                      {#if showAbsoluteResets}
                        <span class="usage-reset-long">Resets {absoluteReset(weeklyReset)}</span><span class="usage-reset-short">R: {absoluteReset(weeklyReset)}</span>
                      {:else}
                        {@const relative = relativeReset(weeklyReset)}
                        <span class="usage-reset-long">{relative.prefix} <strong>{relative.value}</strong></span><span class="usage-reset-short">R: {relative.value}</span>
                      {/if}
                    </button>
                  {:else}
                  {#if weekly.unlimited}
                      <span class="usage-reset"><span class="usage-reset-long">Unlimited</span><span class="usage-reset-short">R: ∞</span></span>
                    {:else if weekly.resetLabel?.trim()}
                      <button class="usage-reset" type="button" aria-label={resetToggleLabel(weekly)} title={resetToggleAction()} onclick={toggleResetDisplay}><span class="usage-reset-long">{resetText(weekly)}</span><span class="usage-reset-short">R: {resetText(weekly)}</span></button>
                    {:else}
                      <span class="usage-reset"><span class="usage-reset-long">{resetText(weekly)}</span><span class="usage-reset-short">R: {resetText(weekly)}</span></span>
                    {/if}
                  {/if}
                </div>
              {/if}
            {:else}
              <div class="usage-detail usage-message">{data?.message?.trim() || 'Usage data is not available.'}</div>
            {/if}
          </div>
          <small class="usage-ring-label">{#if ringsOnly}<span class="usage-provider-icon"><ProviderIcon provider={provider.id} size={11} /></span>{/if}<span class="usage-label-text">{provider.label}</span></small>
        </article>
      {/each}
    </div>
  {/if}
</section>

<style>
  .usage-rings { min-width:0; max-width:100%; overflow:hidden; color:var(--ink); border:1px solid var(--line); border-radius:8px; background:color-mix(in srgb,var(--panel) 34%,transparent); font-size:calc(11px * var(--interface-font-ratio,1)); }
  .usage-toggle { display:grid; grid-template-columns:minmax(0,1fr) auto 16px; align-items:center; width:100%; height:31px; padding:0 7px 0 10px; color:var(--muted); text-align:left; }
  .usage-toggle:hover { color:var(--ink); background:var(--soft); }
  .usage-toggle > span { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-size:calc(9px * var(--interface-font-ratio,1)); font-weight:600; letter-spacing:.09em; }
  .usage-toggle small { font:calc(8px * var(--interface-font-ratio,1)) var(--mono); letter-spacing:.04em; }
  .usage-toggle :global(.usage-chevron) { justify-self:end; transform:rotate(180deg); transition:transform 150ms ease-out; }
  .expanded .usage-toggle :global(.usage-chevron) { transform:rotate(0); }
  .usage-body { display:grid; gap:4px; padding:4px; border-top:1px solid color-mix(in srgb,var(--line) 65%,transparent); }
  .usage-provider { display:grid; grid-template-columns:40px minmax(0,1fr); align-items:center; gap:8px; min-width:0; padding:5px 6px; border:1px solid transparent; border-radius:8px; }
  .usage-provider:hover { background:color-mix(in srgb,var(--accent) 5%,transparent); border-color:color-mix(in srgb,var(--accent) 12%,transparent); }
  .usage-ring-wrap { position:relative; display:grid; width:40px; height:40px; place-items:center; }
  .usage-ring { display:block; width:40px; height:40px; overflow:visible; }
  .usage-provider-icon { display:grid; flex:0 0 auto; width:14px; height:14px; place-items:center; color:var(--ink); }
  .usage-provider-icon :global(.provider-icon) { width:12px; height:12px; }
  .usage-ring circle { fill:none; stroke-linecap:round; }
  .ring-track,.ring-active,.ring-weekly { transform:rotate(-90deg); transform-box:view-box; transform-origin:center; }
  .ring-track { stroke:color-mix(in srgb,var(--muted) 20%,transparent); stroke-width:3; }
  .ring-active { stroke:var(--ring-active-color,var(--accent)); stroke-width:3; transition:stroke-dashoffset .2s ease,stroke .2s ease; }
  .ring-weekly { stroke:color-mix(in srgb,var(--ring-weekly-color,var(--accent)) 45%,var(--muted)); stroke-width:1.5; transition:stroke-dashoffset .2s ease,stroke .2s ease; }
  .ring-value { position:absolute; inset:0; display:grid; place-items:center; color:var(--ink); font:700 9px var(--mono); line-height:1; pointer-events:none; }
  .usage-copy { display:grid; gap:1px; min-width:0; }
  .usage-heading,.usage-detail,.usage-weekly { display:flex; align-items:baseline; gap:6px; min-width:0; white-space:nowrap; }
  .usage-heading { align-items:center; }
  .usage-detail-short,.usage-reset-short { display:none; }
  .usage-heading strong { overflow:hidden; text-overflow:ellipsis; font-size:calc(11.5px * var(--interface-font-ratio,1)); font-weight:550; }
  .usage-status { overflow:hidden; color:var(--muted); text-overflow:ellipsis; font:calc(9.5px * var(--interface-font-ratio,1)) var(--mono); }
  .usage-detail { color:var(--muted); font:calc(9.5px * var(--interface-font-ratio,1)) var(--mono); }
  .usage-reset { min-width:0; overflow:hidden; padding:0; color:inherit; border:0; background:transparent; font:inherit; text-align:left; text-overflow:ellipsis; }
  button.usage-reset { cursor:pointer; }
  .usage-reset strong { color:var(--ink); font-weight:650; }
  button.usage-reset:hover { color:var(--ink); }
  button.usage-reset:focus-visible { outline:1px solid var(--accent); outline-offset:2px; border-radius:2px; }
  .usage-weekly { overflow:hidden; color:color-mix(in srgb,var(--muted) 85%,var(--ink)); font:calc(9px * var(--interface-font-ratio,1)) var(--mono); }
  .usage-weekly span { overflow:hidden; text-overflow:ellipsis; }
  .usage-weekly span:last-child { color:var(--muted); }
  .usage-message { display:block; overflow:hidden; text-overflow:ellipsis; }
  .unavailable .ring-active { stroke:var(--muted); stroke-dasharray:2 4; opacity:.42; }
  .unavailable .ring-value { color:var(--muted); }
  .loading .ring-active { stroke-dasharray:4 4; animation:usage-ring-spin 1.15s linear infinite; }
  .stale .ring-active { stroke:color-mix(in srgb,var(--accent) 58%,var(--muted)); }
  .stale .usage-status { color:color-mix(in srgb,var(--accent) 68%,var(--muted)); }
  .error .ring-active { stroke:#c35b5b; }
  .error .usage-status { color:#c35b5b; }
  .compact { border:0; border-radius:0; background:transparent; }
  .compact .usage-body { justify-items:center; gap:5px; padding:0; border:0; }
  .compact .usage-provider { grid-template-columns:40px; padding:4px; }
  .compact .usage-copy { display:none; }
  .compact .usage-provider:hover { border-color:color-mix(in srgb,var(--accent) 22%,transparent); }
  .usage-ring-label { display:none; }
  .rings-only .usage-body { grid-template-columns:repeat(4,minmax(0,1fr)); justify-items:center; gap:0; padding:6px 4px; }
  .rings-only .usage-provider { grid-template-columns:1fr; grid-template-rows:auto auto; justify-items:center; gap:2px; padding:3px 0; border:0; }
  .rings-only .usage-provider:hover { border-color:transparent; }
  .rings-only .usage-copy { display:none; }
  .rings-only .usage-ring-label { display:flex; align-items:center; justify-content:center; gap:3px; max-width:100%; overflow:hidden; color:var(--muted); font:calc(8px * var(--interface-font-ratio,1)) var(--mono); letter-spacing:.01em; text-align:center; white-space:nowrap; }
  .rings-only .usage-ring-label .usage-provider-icon { width:13px; height:13px; }
  .rings-only .usage-ring-label .usage-provider-icon :global(.provider-icon) { width:9px; height:9px; }
  .rings-only .usage-label-text { overflow:hidden; text-overflow:ellipsis; }
  .horizontal {
    .usage-provider { align-items:start; }
    .usage-ring { margin-top:2px; }
    .usage-copy { gap:2px; }
    .usage-detail,.usage-weekly { display:grid; grid-template-columns:minmax(0,1fr); gap:1px; align-items:baseline; white-space:normal; }
    .usage-detail-long,.usage-reset-long { display:none; }
    .usage-detail-short,.usage-reset-short { display:inline; }
    .usage-reset { overflow:visible; text-overflow:clip; }
  }
  @keyframes usage-ring-spin { to { transform:rotate(360deg); } }
  @media (prefers-reduced-motion:reduce) { .loading .ring-active,.ring-active,.ring-weekly,.usage-toggle :global(.usage-chevron) { animation:none; transition:none; } }
</style>
