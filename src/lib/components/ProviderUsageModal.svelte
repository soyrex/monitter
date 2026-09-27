<script lang="ts">
  import Modal from './Modal.svelte';
  import ProviderIcon from './ProviderIcon.svelte';
  import type { UsageRingData, UsageRingProvider, UsageRingWindow } from './UsageRings.svelte';

  let { open, provider, data, accountLabel, onclose }: {
    open: boolean;
    provider: UsageRingProvider;
    data?: UsageRingData;
    accountLabel?: string;
    onclose: () => void;
  } = $props();

  const providerNames: Record<UsageRingProvider, string> = {
    codex: 'Codex',
    claude: 'Claude',
    gemini: 'Gemini',
    minimax: 'MiniMax',
    'opencode-go': 'OpenCode Go',
  };

  const windows = $derived([data?.active, data?.weekly].filter((window): window is UsageRingWindow => !!window));

  function statusText(value: UsageRingData | undefined): string {
    switch (value?.status) {
      case 'ready': return windows.length ? 'Current' : 'No allowance data';
      case 'stale': return 'Last known values';
      case 'loading': return 'Loading';
      case 'error': return 'Could not load';
      default: return 'Unavailable';
    }
  }

  function usedPercent(window: UsageRingWindow): string {
    if (window.unlimited) return 'Unlimited';
    const value = window.usedPercent;
    return typeof value === 'number' && Number.isFinite(value) && value >= 0 && value <= 100 ? `${value}%` : 'Not reported';
  }

  function remainingPercent(window: UsageRingWindow): string {
    if (window.unlimited) return 'Unlimited';
    const value = window.usedPercent;
    return typeof value === 'number' && Number.isFinite(value) && value >= 0 && value <= 100
      ? `${Number((100 - value).toFixed(2))}%`
      : 'Not reported';
  }

  function dateTime(value: number): string {
    return new Intl.DateTimeFormat(undefined, {
      year: 'numeric', month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit', timeZoneName: 'short',
    }).format(value);
  }

  function resetText(window: UsageRingWindow): string {
    return typeof window.resetsAt === 'number' && Number.isFinite(window.resetsAt)
      ? dateTime(window.resetsAt)
      : window.resetLabel?.trim() || 'Not reported';
  }
</script>

<Modal title={`${providerNames[provider]} usage`} {open} {onclose} globalLayer>
  <div class="provider-usage-details">
    <div class="identity">
      <span class="provider-mark" aria-hidden="true"><ProviderIcon {provider} size={24}/></span>
      <div><strong>{providerNames[provider]}</strong>{#if accountLabel}<span>Profile: {accountLabel}</span>{/if}</div>
      <span class="status" data-status={data?.status ?? 'unavailable'}>{statusText(data)}</span>
    </div>

    {#if windows.length}
      <div class="window-grid">
        {#each windows as window (window.label)}
          <section class="window-card" aria-label={`${window.label} allowance`}>
            <h3>{window.label}</h3>
            <div class="figures">
              <div><span>Used</span><strong>{usedPercent(window)}</strong></div>
              <div><span>Remaining</span><strong>{remainingPercent(window)}</strong></div>
            </div>
            <p>Resets <time>{resetText(window)}</time></p>
          </section>
        {/each}
      </div>
      <p class="calculation-note">Remaining is calculated from the reported used percentage.</p>
    {:else}
      <p class="unavailable">{data?.message?.trim() || 'This provider has not reported allowance details.'}</p>
    {/if}

    {#if data?.message?.trim() && windows.length}<p class="provider-message">{data.message.trim()}</p>{/if}
    {#if typeof data?.updatedAt === 'number' && Number.isFinite(data.updatedAt)}<p class="updated">Last checked {dateTime(data.updatedAt)}</p>{/if}
  </div>
</Modal>

<style>
  .provider-usage-details { display:grid; gap:16px; min-width:0; }
  .identity { display:flex; align-items:center; gap:11px; min-width:0; }
  .provider-mark { display:grid; place-items:center; flex:none; width:42px; height:42px; border:1px solid var(--line); border-radius:11px; background:var(--soft); }
  .identity > div { display:grid; gap:3px; min-width:0; }
  .identity strong { font-size:calc(14px * var(--interface-font-ratio, 1)); }
  .identity div span { overflow-wrap:anywhere; color:var(--muted); font-size:calc(11px * var(--interface-font-ratio, 1)); }
  .status { margin-left:auto; padding:4px 7px; border:1px solid var(--line); border-radius:20px; color:var(--muted); font-size:calc(10px * var(--interface-font-ratio, 1)); white-space:nowrap; }
  .status[data-status="ready"] { color:var(--accent-ink); }
  .window-grid { display:grid; grid-template-columns:repeat(auto-fit,minmax(min(100%,200px),1fr)); gap:10px; }
  .window-card { min-width:0; padding:14px; border:1px solid var(--line); border-radius:10px; background:var(--soft); }
  h3 { margin:0 0 12px; font-size:calc(11px * var(--interface-font-ratio, 1)); font-weight:600; text-transform:uppercase; letter-spacing:.06em; }
  .figures { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:10px; }
  .figures div { display:grid; gap:4px; min-width:0; }
  .figures span { color:var(--muted); font-size:calc(10px * var(--interface-font-ratio, 1)); }
  .figures strong { overflow-wrap:anywhere; font:600 calc(18px * var(--interface-font-ratio, 1))/1.2 var(--mono); }
  .window-card p,.calculation-note,.updated,.provider-message,.unavailable { margin:0; color:var(--muted); font-size:calc(10.5px * var(--interface-font-ratio, 1)); line-height:1.5; }
  .window-card p { margin-top:13px; }
  .window-card time { color:var(--ink); }
  .provider-message,.unavailable { padding:10px 12px; border:1px solid var(--line); border-radius:8px; background:var(--soft); }
</style>
