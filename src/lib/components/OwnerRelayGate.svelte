<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import { loadMobilePairing } from '$lib/controller/pairing-store';
  import { createResumableMobileSession, type ResumableMobileSession } from '$lib/controller/resumable-session';
  import { watchReconnectEvents } from '$lib/controller/reconnect-events';
  import { setOwnerRelayInvoker, noteOwnerRelayReconnectOutcome } from '$lib/lan';
  import { appTheme } from '$lib/app-theme';
  import { surfaceTint } from '$lib/surface-tint';
  import { terminalTheme } from '$lib/terminal-theme';
  import { applyOwnerAppearance, currentOwnerAppearance } from '$lib/owner-appearance';

  let { children }: { children: Snippet } = $props();
  let ready = $state(false);
  let everReady = $state(false);
  let reconnecting = $state(false);
  let permanentProblem = $state(false);
  let status = $state('Connecting to your desktop…');
  let error = $state('');

  onMount(() => {
    let closed = false;
    let session: ResumableMobileSession | null = null;
    let stop: (() => void) | undefined;
    let authorizing = false;
    let authorizationEpoch = 0;
    let applyingAppearance = false;
    let appearanceWrites = 0;
    let appearanceWriteChain: Promise<void> = Promise.resolve();
    let appearanceTimer: ReturnType<typeof setTimeout> | undefined;
    let lastAppearance = '';
    let appearanceRead = false;
    let appearanceRevision = 0;
    let observedAppearance = JSON.stringify(currentOwnerAppearance());
    let appearanceStops: Array<() => void> = [];
    const refreshAppearance = async (current: ResumableMobileSession) => {
      if (appearanceWrites || appearanceTimer || current.getStatus() !== 'connected' || !current.hasOwnerWorkspaceAccess()) return;
      const epoch = authorizationEpoch, revision = appearanceRevision;
      const remote = await current.invokeOwner('get_owner_appearance', {});
      if (closed || epoch !== authorizationEpoch || session !== current || current.getStatus() !== 'connected' || !current.hasOwnerWorkspaceAccess()) return;
      const remoteKey = JSON.stringify(remote);
      if (revision !== appearanceRevision || appearanceWrites || appearanceTimer) {
        if (!appearanceRead) lastAppearance = remoteKey;
        appearanceRead = true;
        sendAppearance();
        return;
      }
      if (remoteKey !== lastAppearance) {
        applyingAppearance = true;
        try { applyOwnerAppearance(remote); lastAppearance = JSON.stringify(currentOwnerAppearance()); }
        finally { applyingAppearance = false; }
      }
      appearanceRead = true;
      watchAppearance();
    };
    const sendAppearance = () => {
      if (closed || applyingAppearance || !appearanceRead || !ready || !session) return;
      const current = session, epoch = authorizationEpoch;
      if (appearanceTimer) clearTimeout(appearanceTimer);
      appearanceTimer = setTimeout(() => {
        appearanceTimer = undefined;
        const value = currentOwnerAppearance();
        const key = JSON.stringify(value);
        if (closed || epoch !== authorizationEpoch || !ready || !appearanceRead || key === lastAppearance || current !== session || current.getStatus() !== 'connected' || !current.hasOwnerWorkspaceAccess()) return;
        const revision = appearanceRevision;
        appearanceWrites += 1;
        appearanceWriteChain = appearanceWriteChain.then(async () => {
          if (closed || epoch !== authorizationEpoch || current !== session || !ready || !appearanceRead || current.getStatus() !== 'connected' || !current.hasOwnerWorkspaceAccess()) return;
          await current.invokeOwner('set_owner_appearance', value);
          if (!closed && epoch === authorizationEpoch && current === session && current.getStatus() === 'connected' && current.hasOwnerWorkspaceAccess() && appearanceRead && revision === appearanceRevision) lastAppearance = key;
        })
          .catch(reason => { if (!closed && epoch === authorizationEpoch && current === session) error = `Could not sync appearance: ${String(reason)}`; })
          .finally(() => { appearanceWrites -= 1; });
      }, 180);
    };
    const watchAppearance = () => {
      if (appearanceStops.length) return;
      const changed = () => {
        const key = JSON.stringify(currentOwnerAppearance());
        if (key === observedAppearance) return;
        observedAppearance = key;
        if (applyingAppearance) return;
        appearanceRevision += 1;
        sendAppearance();
      };
      appearanceStops = [appTheme.subscribe(changed), surfaceTint.subscribe(changed), terminalTheme.subscribe(changed)];
    };
    const connected = async (current: ResumableMobileSession) => {
      if (authorizing) return;
      if (!current.supportsOwnerWorkspace()) {
        status = 'Desktop update required';
        error = 'Update Monitter on your desktop to use the full workspace through the relay. Your existing LAN connection still works.';
        permanentProblem = true;
        everReady = false;
        reconnecting = false;
        return;
      }
      authorizing = true;
      const authorization = ++authorizationEpoch;
      status = 'Waiting for full-workspace approval on your desktop…';
      error = '';
      try {
        // Only request an owner grant when this current transport session does
        // not already have one. The desktop remains authoritative for grants.
        if (!current.hasOwnerWorkspaceAccess()) await current.requestOwnerWorkspace();
        if (closed || authorization !== authorizationEpoch || session !== current || current.getStatus() !== 'connected') return;
        status = 'Opening your workspace…';
        setOwnerRelayInvoker((command, args) => current.invokeOwner(command, args));
        ready = true;
        everReady = true;
        reconnecting = false;
        watchAppearance();
        // Appearance is optional; an authorized workspace can mount immediately.
        void refreshAppearance(current).catch(reason => {
          if (!closed && authorization === authorizationEpoch && session === current && current.getStatus() === 'connected') error = `Could not sync appearance: ${String(reason)}`;
        });
      } catch (reason) {
        if (!closed && authorization === authorizationEpoch && session === current) error = reason instanceof Error ? reason.message : String(reason);
      } finally { if (authorization === authorizationEpoch) authorizing = false; }
    };
    void (async () => {
      try {
        const saved = await loadMobilePairing();
        if (closed) return;
        if (!saved) { window.location.replace('/mobile?owner=1'); return; }
        const current = await createResumableMobileSession({ invitation: saved.invitation, identity: saved.identity });
        if (closed) { current.close(); return; }
        session = current;
        stop = current.subscribe(state => {
          if (closed) return;
          if (state.status === 'connected') { noteOwnerRelayReconnectOutcome(true); void connected(current); }
          else {
            if (state.status === 'closed' || state.status === 'error') noteOwnerRelayReconnectOutcome(false);
            authorizationEpoch += 1;
            authorizing = false;
            ready = false;
            appearanceRead = false;
            if (appearanceTimer) clearTimeout(appearanceTimer);
            appearanceTimer = undefined;
            setOwnerRelayInvoker(null);
            status = state.status === 'awaiting_approval' ? 'Approve this phone on your desktop…' : 'Reconnecting to your desktop…';
            if (everReady && state.status !== 'rejected') reconnecting = true;
            else if (state.error) error = state.error;
            if (state.status === 'rejected') {
              everReady = false;
              reconnecting = false;
              permanentProblem = true;
              error = state.error || 'Desktop declined this connection. Your saved pairing is preserved; retry when ready.';
            }
          }
        });
      } catch (reason) {
        if (!closed) error = reason instanceof Error ? reason.message : String(reason);
      }
    })();
    const appearancePoll = setInterval(() => {
      const current = session, epoch = authorizationEpoch;
      if (current && ready) void refreshAppearance(current).catch(reason => { if (!closed && epoch === authorizationEpoch && session === current && current.getStatus() === 'connected') error = `Could not refresh appearance: ${String(reason)}`; });
    }, 10_000);
    const stopReconnectEvents = watchReconnectEvents(() => {
      const current = session;
      // Reopen even apparently-connected transports: Wi-Fi/cellular changes
      // can leave a WebSocket blackholed until an application heartbeat lands.
      if (!closed && current) {
        void current.reconnectNow().catch(reason => {
          if (!closed && session === current) error = String(reason);
        });
      }
    });
    return () => { closed = true; ready = false; setOwnerRelayInvoker(null); if (appearanceTimer) clearTimeout(appearanceTimer); stopReconnectEvents(); clearInterval(appearancePoll); appearanceStops.forEach(stop => stop()); stop?.(); session?.close(); };
  });
</script>

{#if everReady}
  <div class="owner-relay-content" aria-busy={!ready}>
    {@render children()}
  </div>
  {#if error}<p class="owner-relay-error" role="alert">{error}</p>{/if}
  {#if reconnecting}
    <div class="connection-recovery-spinner" data-testid="connection-recovery-spinner" role="status" aria-live="polite">
      <span class="connection-spinner" aria-hidden="true"></span>
    </div>
  {/if}
{:else}
  <main class="owner-relay-wait">
    <div>
      <h1>Monitter</h1>
      <p role="status">{status}</p>
      {#if error}<p role="alert">{error}</p>{/if}
      {#if permanentProblem}<button type="button" onclick={() => window.location.assign('/mobile?owner=1')}>Reconnect to saved desktop</button>{/if}
    </div>
  </main>
{/if}

<style>
  .owner-relay-error { position:fixed; bottom:16px; left:16px; right:16px; z-index:10001; padding:12px; background:var(--panel, #222); color:#e99b91; font:14px 'IBM Plex Sans', system-ui, sans-serif; }
  .owner-relay-wait { min-height: 100dvh; display: grid; place-items: center; padding: 24px; box-sizing: border-box; background: var(--paper, #151515); color: var(--ink, #eee); font: 14px 'IBM Plex Sans', system-ui, sans-serif; }
  .owner-relay-wait > div { width: min(100%, 420px); }
  h1 { font-size: 28px; margin: 0 0 12px; }
  p { line-height: 1.5; }
  [role=alert] { color: #e99b91; }
  button { margin-top: 14px; padding: 11px 15px; border: 1px solid var(--line, #555); border-radius: 8px; background: var(--panel, #222); color: inherit; font: inherit; }
  .connection-recovery-spinner { position:fixed; right:16px; bottom:calc(16px + env(safe-area-inset-bottom)); z-index:10000; pointer-events:none; color:var(--ink, #eee); }
  .connection-spinner { display:block; width:18px; height:18px; border:2px solid color-mix(in srgb, currentColor 25%, transparent); border-top-color:currentColor; border-radius:50%; animation:connection-spin .8s linear infinite; }
  @keyframes connection-spin { to { transform:rotate(360deg); } }
</style>
