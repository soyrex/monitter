<script lang="ts">
  import { onMount } from 'svelte';
  import { getLanServerInfo, isLanBrowser, type LanServerInfo } from '$lib/lan';
  let info = $state<LanServerInfo | null>(null), error = $state(''), copied = $state('');
  const browser = isLanBrowser();
  onMount(() => { if (!browser) void getLanServerInfo().then(value => info = value).catch(reason => error = String(reason)); });
  async function copy(value: string, label: string) {
    try { await navigator.clipboard.writeText(value); copied = label; }
    catch { error = 'Select and copy the address or code below.'; }
  }
</script>
<section>
  <h2>Open Monitter on your network</h2>
  {#if browser}
    <p>You are connected to Monitter at {location.host}. Work runs on the host Mac.</p>
  {:else if info}
    {#if info.error}<p role="alert">{info.error}</p>{:else}
      <p>Open one of these addresses from a device on the same LAN. Keep the desktop app running.</p>
      {#each info.urls as url}<div class="address"><a href={url} target="_blank" rel="noreferrer">{url}</a><button onclick={() => copy(info!.accessCodeRequired ? `${url}#access=${encodeURIComponent(info!.token)}` : url, 'Access link copied')}>Copy access link</button></div>{/each}
      {#if info.accessCodeRequired}
      <label>Access code<input class="access-code" readonly value={info.token} onclick={event => event.currentTarget.select()} /></label>
      <p class="hint">Enter this six-digit code on your phone. It changes when Monitter restarts. Five incorrect guesses temporarily lock LAN access for five minutes.</p>
      <p class="hint">The code grants control of this workspace, including terminals. HTTP traffic is unencrypted; use a trusted LAN.</p>
      {:else}
      <p>Access code protection is temporarily disabled. Anyone on this LAN can access the workspace and terminals.</p>
      <p class="hint">Access-code support is retained for later. HTTP traffic is unencrypted; use a trusted LAN.</p>
      {/if}
      {#if copied}<p role="status">{copied}</p>{/if}
    {/if}
  {:else if !error}<p>Loading server address…</p>{/if}
  {#if error}<p role="alert">{error}</p>{/if}
</section>
<style>
 .access-code{font-size:28px;font-variant-numeric:tabular-nums;letter-spacing:.25em}
 section{padding:24px;border:1px solid var(--border,#444);border-radius:12px}h2{margin-top:0}p{line-height:1.6}.address{display:flex;flex-wrap:wrap;gap:12px;align-items:center;margin:12px 0}a{color:var(--accent,#3f9d6a)}button{font:inherit;padding:8px 12px;border:1px solid var(--border,#444);border-radius:6px;cursor:pointer;background:transparent;color:inherit}label{display:grid;gap:8px;margin-top:24px}input{font:inherit;color:inherit;background:transparent;border:1px solid var(--border,#444);border-radius:6px;padding:10px;width:100%;box-sizing:border-box}.hint{font-size:13px;color:var(--muted,#999)}[role=alert]{color:#dc7373}
</style>
