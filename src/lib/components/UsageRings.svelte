<script lang="ts">
  import { ChevronDown } from '@lucide/svelte';
  import { onMount } from 'svelte';

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
  const storageKey = 'monitter.sidebar-usage-expanded.v1';

  let {
    usage = {},
    compact = false,
    expanded = $bindable(true),
    class: className = '',
  }: { usage?: UsageRingMap; compact?: boolean; expanded?: boolean; class?: string } = $props();
  let showAbsoluteResets = $state(false);
  let currentTime = $state(Date.now());

  onMount(() => {
    try {
      const stored = localStorage.getItem(storageKey);
      if (stored === 'true' || stored === 'false') expanded = stored === 'true';
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

  function toggle() {
    expanded = !expanded;
    try { localStorage.setItem(storageKey, String(expanded)); } catch { /* The live state still works. */ }
  }

  function percent(value: number | null | undefined): number | null {
    return typeof value === 'number' && Number.isFinite(value) ? Math.max(0, Math.min(100, value)) : null;
  }

  function percentText(value: number | null | undefined, unlimited = false): string {
    if (unlimited) return '∞';
    const normalized = percent(value);
    return normalized === null ? '—' : `${Math.round(normalized)}%`;
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

<section class:compact class:expanded class={`usage-rings ${className}`.trim()} aria-label="Provider usage">
  {#if !compact}
    <button class="usage-toggle" type="button" aria-expanded={expanded} aria-controls="sidebar-usage-body" onclick={toggle}>
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
          <svg class="usage-ring" viewBox="0 0 40 40" role="img" aria-label={`${provider.label}: ${hasValue ? `${percentText(active?.usedPercent)} used in ${active?.label}` : stateLabel(data)}`}>
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
            <text class="ring-value" x="20" y="21" text-anchor="middle">{hasValue ? percentText(active?.usedPercent, active?.unlimited).replace('%', '') : provider.mark}</text>
          </svg>
          <div class="usage-copy">
            <div class="usage-heading"><strong>{provider.label}</strong><span class="usage-status">{status === 'ready' ? active?.label ?? stateLabel(data) : stateLabel(data)}</span></div>
            {#if hasValue}
              <div class="usage-detail">
                <span>{active?.unlimited ? 'Unlimited' : `${percentText(active?.usedPercent)} used.`}</span>
                {#if resetTimestamp(active) !== null}
                  {@const activeReset = resetTimestamp(active)!}
                  <button class="usage-reset" type="button" aria-label={resetToggleLabel(active)} title={resetToggleAction()} onclick={toggleResetDisplay}>
                    {#if showAbsoluteResets}
                      Resets {absoluteReset(activeReset)}
                    {:else}
                      {@const relative = relativeReset(activeReset)}
                      {relative.prefix} <strong>{relative.value}</strong>
                    {/if}
                  </button>
                {:else}
                  {#if active?.resetLabel?.trim()}
                    <button class="usage-reset" type="button" aria-label={resetToggleLabel(active)} title={resetToggleAction()} onclick={toggleResetDisplay}>{resetText(active)}</button>
                  {:else}
                    <span class="usage-reset" title={resetText(active)}>{resetText(active)}</span>
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
                        Resets {absoluteReset(weeklyReset)}
                      {:else}
                        {@const relative = relativeReset(weeklyReset)}
                        {relative.prefix} <strong>{relative.value}</strong>
                      {/if}
                    </button>
                  {:else}
                    {#if weekly.unlimited}
                      <span class="usage-reset">Unlimited</span>
                    {:else if weekly.resetLabel?.trim()}
                      <button class="usage-reset" type="button" aria-label={resetToggleLabel(weekly)} title={resetToggleAction()} onclick={toggleResetDisplay}>{resetText(weekly)}</button>
                    {:else}
                      <span class="usage-reset">{resetText(weekly)}</span>
                    {/if}
                  {/if}
                </div>
              {/if}
            {:else}
              <div class="usage-detail usage-message">{data?.message?.trim() || 'Usage data is not available.'}</div>
            {/if}
          </div>
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
  .usage-ring { display:block; width:40px; height:40px; overflow:visible; transform:rotate(-90deg); }
  .usage-ring circle { fill:none; stroke-linecap:round; }
  .ring-track { stroke:color-mix(in srgb,var(--muted) 20%,transparent); stroke-width:3; }
  .ring-active { stroke:var(--accent); stroke-width:3; transition:stroke-dashoffset .2s ease; }
  .ring-weekly { stroke:color-mix(in srgb,var(--accent) 45%,var(--muted)); stroke-width:1.5; transition:stroke-dashoffset .2s ease; }
  .ring-value { fill:var(--ink); font:700 9px var(--mono); transform:rotate(90deg); transform-origin:20px 20px; }
  .usage-copy { display:grid; gap:1px; min-width:0; }
  .usage-heading,.usage-detail,.usage-weekly { display:flex; align-items:baseline; gap:6px; min-width:0; white-space:nowrap; }
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
  .unavailable .ring-value { fill:var(--muted); }
  .loading .ring-active { stroke-dasharray:4 4; animation:usage-ring-spin 1.15s linear infinite; transform-origin:20px 20px; }
  .stale .ring-active { stroke:color-mix(in srgb,var(--accent) 58%,var(--muted)); }
  .stale .usage-status { color:color-mix(in srgb,var(--accent) 68%,var(--muted)); }
  .error .ring-active { stroke:#c35b5b; }
  .error .usage-status { color:#c35b5b; }
  .compact { border:0; border-radius:0; background:transparent; }
  .compact .usage-body { justify-items:center; gap:5px; padding:0; border:0; }
  .compact .usage-provider { grid-template-columns:40px; padding:4px; }
  .compact .usage-copy { display:none; }
  .compact .usage-provider:hover { border-color:color-mix(in srgb,var(--accent) 22%,transparent); }
  @keyframes usage-ring-spin { to { transform:rotate(360deg); } }
  @media (prefers-reduced-motion:reduce) { .loading .ring-active,.ring-active,.ring-weekly,.usage-toggle :global(.usage-chevron) { animation:none; transition:none; } }
</style>
