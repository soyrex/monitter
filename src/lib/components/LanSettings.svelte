<script lang="ts">
  import { onMount } from 'svelte';
  import { getLanServerInfo, isLanBrowser, isOwnerRelayBrowser, type LanServerInfo } from '$lib/lan';
  let info = $state<LanServerInfo | null>(null), error = $state(''), copied = $state('');
  let browserCertificateAvailable = $state(false), browserFingerprint = $state('');
  const browser = isLanBrowser();
  onMount(() => {
    if (browser && !isOwnerRelayBrowser()) {
      void (async () => {
        try {
          const response = await fetch('/api/lan-ca.crt', { cache: 'no-store' });
          if (!response.ok) return;
          const pem = await response.text();
          const base64 = pem.replace(/-----BEGIN CERTIFICATE-----|-----END CERTIFICATE-----|\s/g, '');
          const der = Uint8Array.from(atob(base64), character => character.charCodeAt(0));
          browserFingerprint = await certificateFingerprint(der);
          browserCertificateAvailable = true;
        } catch { browserCertificateAvailable = false; }
      })();
    } else if (!browser) {
      void getLanServerInfo().then(value => info = value).catch(reason => error = String(reason));
    }
  });
  async function certificateFingerprint(der: Uint8Array): Promise<string> {
    let digest: Uint8Array;
    if (globalThis.crypto?.subtle) {
      const source = new Uint8Array(new ArrayBuffer(der.byteLength));
      source.set(der);
      digest = new Uint8Array(await crypto.subtle.digest('SHA-256', source.buffer));
    }
    else digest = sha256Fallback(der);
    return Array.from(digest, byte => byte.toString(16).padStart(2, '0').toUpperCase()).join(':');
  }
  function sha256Fallback(input: Uint8Array): Uint8Array {
    const k = new Uint32Array([
      0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
      0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
      0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
      0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
      0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
      0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
      0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
      0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2
    ]);
    const bitLength = input.length * 8;
    const paddedLength = Math.ceil((input.length + 9) / 64) * 64;
    const bytes = new Uint8Array(paddedLength);
    bytes.set(input); bytes[input.length] = 0x80;
    const view = new DataView(bytes.buffer);
    view.setUint32(paddedLength - 8, Math.floor(bitLength / 0x100000000));
    view.setUint32(paddedLength - 4, bitLength >>> 0);
    const h = new Uint32Array([0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19]);
    const w = new Uint32Array(64), rotr = (x: number, n: number) => (x >>> n) | (x << (32 - n));
    for (let offset = 0; offset < paddedLength; offset += 64) {
      for (let i = 0; i < 16; i++) w[i] = view.getUint32(offset + i * 4);
      for (let i = 16; i < 64; i++) {
        const x = w[i - 15], y = w[i - 2];
        const s0 = rotr(x, 7) ^ rotr(x, 18) ^ (x >>> 3), s1 = rotr(y, 17) ^ rotr(y, 19) ^ (y >>> 10);
        w[i] = (w[i - 16] + s0 + w[i - 7] + s1) >>> 0;
      }
      let [a,b,c,d,e,f,g,hh] = h;
      for (let i = 0; i < 64; i++) {
        const s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25), ch = (e & f) ^ (~e & g);
        const t1 = (hh + s1 + ch + k[i] + w[i]) >>> 0;
        const s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22), maj = (a & b) ^ (a & c) ^ (b & c);
        const t2 = (s0 + maj) >>> 0;
        [hh,g,f,e,d,c,b,a] = [g,f,e,(d + t1) >>> 0,c,b,a,(t1 + t2) >>> 0];
      }
      h[0]=(h[0]+a)>>>0; h[1]=(h[1]+b)>>>0; h[2]=(h[2]+c)>>>0; h[3]=(h[3]+d)>>>0;
      h[4]=(h[4]+e)>>>0; h[5]=(h[5]+f)>>>0; h[6]=(h[6]+g)>>>0; h[7]=(h[7]+hh)>>>0;
    }
    const output = new Uint8Array(32), outputView = new DataView(output.buffer);
    h.forEach((value, index) => outputView.setUint32(index * 4, value));
    return output;
  }
  async function copy(value: string, label: string) {
    try { await navigator.clipboard.writeText(value); copied = label; }
    catch { error = 'Select and copy the address or code below.'; }
  }
  function exportRootCertificate() {
    if (!info?.localCaPem) return;
    const url = URL.createObjectURL(new Blob([info.localCaPem], { type: 'application/x-x509-ca-cert' }));
    const link = document.createElement('a');
    link.href = url;
    link.download = 'monitter-local-root-ca.crt';
    link.click();
    URL.revokeObjectURL(url);
  }
  const httpsUrls = $derived(info?.urls.filter(url => url.startsWith('https://')) ?? []);
  const httpUrls = $derived(info?.urls.filter(url => url.startsWith('http://')) ?? []);
</script>
<section>
  <h2>Open Monitter on your network</h2>
  {#if browser}
    <p>You are connected to Monitter at {location.host}. Work runs on the host Mac.</p>
    {#if isOwnerRelayBrowser()}
      <p class="hint">This paired iPhone workspace uses the encrypted relay. Voice recording here requires a separate iPhone microphone integration.</p>
    {:else}
    <p class="hint">Browser recording requires a secure HTTPS connection and a trusted Monitter certificate. If this page is open over HTTP, open the HTTPS address from desktop Settings and complete certificate trust on this device first.</p>
    {#if browserCertificateAvailable}
      <p><a href="/api/lan-ca.crt" download="monitter-local-root-ca.crt">Download Monitter public root certificate (.crt)</a></p>
      <p class="fingerprint"><strong>Certificate SHA-256 fingerprint:</strong> <code>{browserFingerprint}</code></p>
      <p class="hint">Compare this fingerprint with the one in native desktop Settings through a separate trusted channel (for example, in person). An HTTP page can be spoofed, so do not trust a fingerprint shown here until it matches the desktop.</p>
      <p class="hint">On iOS, after installing the certificate profile, enable full trust for it in Settings → General → About → Certificate Trust Settings, then reload HTTPS.</p>
    {/if}
    {/if}
  {:else if info}
    {#if info.error}<p role="alert">{info.error}</p>{:else}
      <p>Open an address from a device on the same LAN. Keep the desktop app running.</p>
      {#if httpsUrls.length}
        <h3>HTTPS</h3>
        {#each httpsUrls as url}<div class="address"><a href={url} target="_blank" rel="noreferrer">{url}</a><button onclick={() => copy(info!.accessCodeRequired ? `${url}#access=${encodeURIComponent(info!.token)}` : url, 'Access link copied')}>Copy access link</button></div>{/each}
        {#if info.localCaPem && info.localCaFingerprint}
          <div class="trust-box">
            <p>To trust HTTPS on another device, download this public root certificate and install it as a trusted certificate on that device.</p>
            <button onclick={exportRootCertificate}>Download root certificate (.crt)</button>
            <p class="fingerprint"><strong>SHA-256 fingerprint:</strong> <code>{info.localCaFingerprint}</code></p>
            <p class="hint">Before trusting it, compare this fingerprint with the one shown by Monitter on this desktop. After trust is installed, reload the HTTPS address.</p>
          </div>
        {/if}
      {/if}
      {#if httpUrls.length}
        <h3>HTTP</h3>
        {#each httpUrls as url}<div class="address"><a href={url} target="_blank" rel="noreferrer">{url}</a><button onclick={() => copy(info!.accessCodeRequired ? `${url}#access=${encodeURIComponent(info!.token)}` : url, 'Access link copied')}>Copy access link</button></div>{/each}
        <p class="hint">HTTP traffic is unencrypted. Use HTTPS when available; on HTTP, people on the network may be able to read or alter traffic.</p>
      {/if}
      {#if info.accessCodeRequired}
      <label>Access code<input class="access-code" readonly value={info.token} onclick={event => event.currentTarget.select()} /></label>
      <p class="hint">Enter this six-digit code on your phone. It changes when Monitter restarts. Five incorrect guesses temporarily lock LAN access for five minutes.</p>
      <p class="hint">The code grants control of this workspace, including terminals.</p>
      {:else}
      <p>Access code protection is temporarily disabled. Anyone on this LAN can access the workspace and terminals.</p>
      <p class="hint">Access-code support is retained for later.</p>
      {/if}
      {#if copied}<p role="status">{copied}</p>{/if}
    {/if}
  {:else if !error}<p>Loading server address…</p>{/if}
  {#if error}<p role="alert">{error}</p>{/if}
</section>
<style>
 .access-code{font-size:28px;font-variant-numeric:tabular-nums;letter-spacing:.25em}
 section{padding:24px;border:1px solid var(--border,#444);border-radius:12px}h2{margin-top:0}h3{margin:22px 0 8px}.address{display:flex;flex-wrap:wrap;gap:12px;align-items:center;margin:12px 0}a{color:var(--accent,#3f9d6a)}button{font:inherit;padding:8px 12px;border:1px solid var(--border,#444);border-radius:6px;cursor:pointer;background:transparent;color:inherit}.trust-box{margin:20px 0;padding:16px;border:1px solid var(--border,#444);border-radius:8px}.fingerprint{overflow-wrap:anywhere}.fingerprint code{font-size:12px}.hint{font-size:13px;color:var(--muted,#999)}[role=alert]{color:#dc7373}
</style>
