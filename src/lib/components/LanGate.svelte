<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import { accessKey, isLanBrowser, lanInvoke, setAccessKey, rememberAccessKey, lanAccessCodeRequired } from '$lib/lan';
  let { children }: { children: Snippet } = $props();
  let ready = $state(!isLanBrowser()), key = $state(''), error = $state(''), pending = $state(false);
  let checking = $state(isLanBrowser());
  let legacyAccess = $state(false);
  let pairingPin = $state('');
  const pairingAvailable = typeof window !== 'undefined' && window.isSecureContext && !!window.crypto?.subtle;
  function pair() {
    if (!pairingAvailable || !/^\d{9}$/.test(pairingPin.replace(/[\s-]/g, ''))) return;
    // Keep the one-time PIN out of URLs, browser history and persistent storage.
    try {
      sessionStorage.setItem('monitter-pairing-pin', pairingPin.replace(/[\s-]/g, ''));
      window.location.assign('/mobile?owner=1');
    } catch { error = 'Could not carry the PIN to pairing. Open /mobile?owner=1 and enter it there.'; }
  }
  async function useLegacyAccess() {
    legacyAccess = true; error = '';
    if (!await lanAccessCodeRequired()) ready = true;
    else if (key) await connect();
  }
  async function connect() {
    if (pending) return;
    const connectingKey = key.trim();
    pending = true; error = '';
    try {
      setAccessKey(connectingKey);
      await lanInvoke('get_command_capabilities');
      rememberAccessKey(connectingKey);
      ready = true;
    }
    catch (reason) { error = String(reason instanceof Error ? reason.message : reason); }
    finally { pending = false; }
  }
  onMount(() => {
    if (!isLanBrowser()) return;
    const fragment = new URLSearchParams(location.hash.slice(1));
    key = fragment.get('access') ?? accessKey();
    if (fragment.has('access')) history.replaceState(null, '', location.pathname + location.search);
    const expired = () => { ready = false; key = ''; error = 'Reconnect with the current six-digit code from the desktop app.'; };
    window.addEventListener('monitter-lan-unauthorized', expired);
    let mounted = true;
    if (pairingAvailable) {
      checking = false;
      return () => { mounted = false; window.removeEventListener('monitter-lan-unauthorized', expired); };
    }
    legacyAccess = true;
    void lanAccessCodeRequired().then(required => {
      if (!mounted) return;
      checking = false;
      if (!required) ready = true;
      else if (key) void connect();
    });
    return () => { mounted = false; window.removeEventListener('monitter-lan-unauthorized', expired); };
  });
</script>
{#if ready}{@render children()}{:else if checking}
  <main class="lan-login"><p role="status">Connecting to Monitter…</p></main>
{:else}
  <main class="lan-login">
    <form onsubmit={event => { event.preventDefault(); if (legacyAccess) void connect(); else pair(); }}>
      <h1>Monitter</h1>
      <p>Connect to the Monitter app on this Mac.</p>
      {#if legacyAccess}
      <label>LAN access code<input type="text" inputmode="numeric" pattern={'[0-9]{6}'} maxlength="6" bind:value={key} autocomplete="one-time-code" placeholder="000000" required /></label>
      <p class="hint">Enter the six-digit code from Settings → LAN access.</p>
      <button disabled={pending || !/^[0-9]{6}$/.test(key)}>{pending ? 'Connecting…' : 'Connect'}</button>
      <button type="button" onclick={()=>legacyAccess=false}>Use pairing PIN</button>
      {:else}
      {#if pairingAvailable}
      <label>Pairing PIN<input inputmode="numeric" autocomplete="one-time-code" maxlength="11" bind:value={pairingPin} placeholder="123 456 789" required /></label>
      <p class="hint">Use the one-time PIN from Settings → Pair a phone, then approve this device on your desktop. It will reconnect without another code.</p>
      <button disabled={!/^\d{9}$/.test(pairingPin.replace(/[\s-]/g,''))}>Pair this device</button>
      {:else}
      <p class="hint">Pairing needs the iPhone app or a secure HTTPS connection. This plain HTTP LAN address can still use its LAN access code.</p>
      {/if}
      <button type="button" onclick={()=>void useLegacyAccess()}>Use LAN access code</button>
      {/if}
      {#if error}<p role="alert">{error}</p>{/if}
    </form>
  </main>
{/if}
<style>
  .lan-login{min-height:100dvh;display:grid;place-items:center;background:#191a17;color:#e7e8e1;font:16px 'IBM Plex Sans',sans-serif;padding:24px;box-sizing:border-box}
  form{width:min(100%,400px)} h1{font-size:32px} p{line-height:1.5}label{display:grid;gap:8px}input{background:#262822;border:1px solid #555b50;color:inherit;padding:12px;border-radius:8px;font:inherit}button{padding:12px 24px;border:0;border-radius:8px;background:#3f9d6a;color:white;font:inherit;cursor:pointer}button:disabled{opacity:.5}.hint{font-size:14px;color:#a8afa2}[role=alert]{color:#f5a19a}
</style>
