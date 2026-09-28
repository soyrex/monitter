<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { getBridge } from '$lib/bridge';
  import { isLanBrowser } from '$lib/lan';
  import { isAgyHeadlessLaunch } from '$lib/agent-permissions';
  import type { Agent, Host, MailAccount, MailConfig, MailConnectStart, MailConnectStatus } from '$lib/types';

  let { agents = [], hosts = [] }: { agents?: Agent[]; hosts?: Host[] } = $props();
  const nativeAvailable = $derived(!isLanBrowser() && getBridge().available);
  let config = $state<MailConfig | null>(null);
  let loading = $state(true);
  let saving = $state(false);
  let refreshing = $state(false);
  let starting = $state(false);
  let error = $state('');
  let notice = $state('');
  let clientId = $state('');
  let clientSecret = $state('');
  let showSetup = $state(false);
  let attempt = $state<MailConnectStart | null>(null);
  let connectStatus = $state<MailConnectStatus | null>(null);
  let disposed = false;
  let pollGeneration = 0;
  let pendingAssignment = $state<{ accountId: string; agentIds: string[] } | null>(null);
  const eligibleAgents = $derived(agents.filter(agent => {
    const host = hosts.find(item => item.id === agent.hostId);
    const launch = agent.acp;
    const agy = isAgyHeadlessLaunch(launch) || Boolean(launch && (launch.command.split(/[\\/]/).at(-1) === 'monitter-agy-acp' || launch.args.some(arg => arg.endsWith('agy-acp-bridge.mjs'))));
    return !agent.internal && host?.kind === 'local' && ['codex', 'claude', 'acp'].includes(agent.provider) && !agy;
  }));

  async function reload() {
    if (!nativeAvailable) { loading = false; return; }
    error = '';
    try { config = await getBridge().getMailConfig(); clientId = config.clientId; }
    catch (reason) { error = `Could not load Mail settings: ${String(reason)}`; }
    finally { loading = false; }
  }
  onMount(() => { void reload(); });
  onDestroy(() => {
    disposed = true;
    pollGeneration += 1;
    if (attempt) void getBridge().cancelMailConnect(attempt.attemptId).catch(() => {});
  });

  async function saveClient() {
    if (!config) return;
    const secret = clientSecret;
    clientSecret = '';
    saving = true; error = ''; notice = '';
    try {
      config = await getBridge().configureGmail(config.revision, clientId.trim(), secret || undefined);
      clientId = config.clientId;
      notice = 'Google app settings saved.';
      showSetup = false;
    } catch (reason) { error = `Could not save Google app settings: ${String(reason)}`; }
    finally { clientSecret = ''; saving = false; }
  }

  async function connect() {
    if (starting || attempt) return;
    starting = true;
    error = ''; notice = ''; connectStatus = null;
    try {
      const started = await getBridge().startMailConnect();
      attempt = started;
      const generation = ++pollGeneration;
      void pollConnect(started, generation);
    } catch (reason) { error = `Could not start Google sign-in: ${String(reason)}`; }
    finally { starting = false; }
  }

  const pause = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));
  async function pollConnect(started: MailConnectStart, generation: number) {
    try {
      while (!disposed && generation === pollGeneration && attempt?.attemptId === started.attemptId && Date.now() < started.expiresAt) {
        await pause(900);
        if (disposed || generation !== pollGeneration || attempt?.attemptId !== started.attemptId) break;
        const status = await getBridge().getMailConnectStatus(started.attemptId);
        if (disposed || generation !== pollGeneration || attempt?.attemptId !== started.attemptId) break;
        connectStatus = status;
        if (status.status === 'connected') {
          const latest = await getBridge().getMailConfig();
          if (disposed || generation !== pollGeneration || attempt?.attemptId !== started.attemptId) break;
          config = latest;
          clientId = latest.clientId;
          attempt = null; notice = status.account ? `Connected ${status.account.email}.` : 'Google account connected.';
          break;
        }
        if (status.status === 'error') { error = status.error || 'Google sign-in failed.'; attempt = null; break; }
        if (status.status === 'cancelled') { attempt = null; break; }
      }
      if (!disposed && generation === pollGeneration && attempt?.attemptId === started.attemptId && Date.now() >= started.expiresAt) {
        const message = 'Google sign-in timed out. Start again when ready.';
        connectStatus = { status: 'error', error: message, account: null };
        attempt = null;
        error = message;
        void getBridge().cancelMailConnect(started.attemptId).catch(() => {});
      }
    } catch (reason) {
      if (!disposed && generation === pollGeneration) {
        error = `Could not check Google sign-in: ${String(reason)}`; attempt = null;
        void getBridge().cancelMailConnect(started.attemptId).catch(() => {});
      }
    }
  }

  async function cancelConnect() {
    const current = attempt;
    if (!current) return;
    pollGeneration += 1;
    attempt = null; error = '';
    try { await getBridge().cancelMailConnect(current.attemptId); connectStatus = { status: 'cancelled', error: null, account: null }; }
    catch (reason) { error = `Could not cancel Google sign-in: ${String(reason)}`; }
  }

  async function refreshSettings() {
    if (loading || saving || refreshing) return;
    const unchangedClientId = clientId === config?.clientId;
    refreshing = true;
    try {
      const latest = await getBridge().getMailConfig();
      config = latest;
      if (unchangedClientId) clientId = latest.clientId;
      error = '';
      notice = 'Mail settings refreshed.';
    } catch (reason) { error = `Could not refresh Mail settings: ${String(reason)}`; }
    finally { refreshing = false; }
  }

  function toggleAgent(account: MailAccount, agentId: string) {
    if (!config || saving) return;
    const currentIds = pendingAssignment?.accountId === account.id ? pendingAssignment.agentIds : account.agentIds;
    const agentIds = currentIds.includes(agentId) ? currentIds.filter(id => id !== agentId) : [...currentIds, agentId];
    pendingAssignment = { accountId: account.id, agentIds };
    void updateAgents(account, agentIds);
  }
  async function updateAgents(account: MailAccount, agentIds: string[]) {
    if (!config) return;
    saving = true; error = ''; notice = '';
    try {
      config = await getBridge().setMailAccountAgents(config.revision, account.id, agentIds);
      pendingAssignment = null;
      notice = 'Saved. Start a new session if this agent does not yet have Mail tools. Removing access takes effect immediately.';
    }
    catch (reason) { error = `Could not update agent access: ${String(reason)}`; }
    finally { saving = false; }
  }

  async function refreshAndRetryAssignment() {
    if (!pendingAssignment || saving) return;
    const pending = pendingAssignment;
    saving = true; error = '';
    try {
      config = await getBridge().getMailConfig(); clientId = config.clientId;
      config = await getBridge().setMailAccountAgents(config.revision, pending.accountId, pending.agentIds);
      pendingAssignment = null;
      notice = 'Saved. Start a new session if this agent does not yet have Mail tools. Removing access takes effect immediately.';
    } catch (reason) { error = `Could not refresh and retry agent access: ${String(reason)}`; }
    finally { saving = false; }
  }

  async function disconnect(account: MailAccount) {
    if (!config || saving) return;
    saving = true; error = ''; notice = '';
    try { config = await getBridge().disconnectMailAccount(config.revision, account.id); notice = `Disconnected ${account.email}.`; }
    catch (reason) { error = `Could not disconnect ${account.email}: ${String(reason)}`; }
    finally { saving = false; }
  }
</script>

<section class="mail-card" aria-labelledby="mail-heading" aria-busy={loading || saving || starting}>
  <div class="card-heading">
    <div><h3 id="mail-heading">Mail</h3><p>Connect Gmail for read-only search and message access through an agent’s MCP tools.</p></div>
    {#if config?.accounts.length}<span class="connected-count">{config.accounts.length} connected</span>{/if}
  </div>

  {#if !nativeAvailable}
    <p class="hint">Mail connections are managed on this desktop and are never available from LAN or shared browser clients.</p>
  {:else if loading}
    <p class="hint">Loading Mail settings…</p>
  {:else if !config}
    <p class="hint">{error || 'Mail settings are unavailable.'}</p><button type="button" onclick={() => { loading = true; void reload(); }}>Retry</button>
  {:else}
    {#if error}<p class="error" role="alert">{error}</p><button class="refresh-settings" type="button" disabled={saving || loading || refreshing} onclick={() => void refreshSettings()}>{refreshing ? 'Refreshing…' : 'Refresh settings'}</button>{/if}
    {#if notice}<p class="notice" role="status">{notice}</p>{/if}

    {#each config.accounts as account (account.id)}
      <article class="account">
        <header><div><strong>{account.email}</strong><small>Google Gmail · Connected {new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' }).format(account.connectedAt)}</small></div><button class="disconnect" type="button" disabled={saving} onclick={() => void disconnect(account)}>Disconnect</button></header>
        <fieldset>
          <legend>Available to these local agents</legend>
          {#each eligibleAgents as agent (agent.id)}
            <label><input type="checkbox" checked={pendingAssignment?.accountId === account.id ? pendingAssignment.agentIds.includes(agent.id) : account.agentIds.includes(agent.id)} disabled={saving} onchange={() => toggleAgent(account, agent.id)} />{agent.name}</label>
          {:else}<p class="hint">Create a local Codex, Claude, or supported ACP agent to assign Mail access.</p>{/each}
        </fieldset>
        {#if (pendingAssignment?.accountId === account.id ? pendingAssignment.agentIds : account.agentIds).length === 0}<p class="hint">No agents can access this account until you assign one.</p>{/if}
        {#if pendingAssignment?.accountId === account.id && error}<button type="button" disabled={saving} onclick={() => void refreshAndRetryAssignment()}>Refresh and retry access</button>{/if}
      </article>
    {:else}
      <p class="empty">No Gmail accounts connected.</p>
    {/each}

    {#if attempt}
      <div class="pending" role="status"><span>Waiting for Google sign-in to finish…</span><button type="button" disabled={saving} onclick={() => void cancelConnect()}>Cancel</button></div>
    {:else}
      <button class="primary" type="button" disabled={saving || starting || !config.clientId} onclick={() => void connect()}>{starting ? 'Opening Google sign-in…' : config.accounts.length ? 'Connect another Gmail account' : 'Connect Gmail'}</button>
    {/if}

    <details class="setup" open={showSetup} ontoggle={(event) => showSetup = event.currentTarget.open}>
      <summary>{config.clientId ? 'Google app setup' : 'Set up your Google OAuth app'}</summary>
      <p class="hint">Create an OAuth client for this Monitter desktop app and choose the <strong>Desktop app</strong> application type. Enter its client ID and, when required, its client secret. The secret is write-only and stored in the macOS Keychain. Monitter opens Google sign-in in your system browser; refresh tokens stay in the Keychain. Follow <a href="https://developers.google.com/identity/protocols/oauth2/native-app" target="_blank" rel="noreferrer">Google's native app setup guide</a>. Public Gmail distribution requires <a href="https://support.google.com/cloud/answer/13463073" target="_blank" rel="noreferrer">OAuth app verification</a>.</p>
      <label>OAuth client ID<input bind:value={clientId} autocomplete="off" spellcheck="false" placeholder="…apps.googleusercontent.com" /></label>
      <label>OAuth client secret <small>{config.clientSecretConfigured ? 'Saved; leave blank to keep the current secret.' : 'Stored securely and never shown again.'}</small><input type="password" bind:value={clientSecret} autocomplete="new-password" /></label>
      <div class="actions"><button type="button" disabled={saving} onclick={() => { clientId = config?.clientId ?? ''; clientSecret = ''; showSetup = false; }}>Cancel</button><button type="button" class="primary" disabled={saving || !clientId.trim()} onclick={() => void saveClient()}>{saving ? 'Saving…' : 'Save app settings'}</button></div>
    </details>
    <p class="footnote">Read-only access. Agents can search Gmail and read messages; they cannot send, delete, or change mail. Fetched messages become part of the selected agent's context.</p>
  {/if}
</section>

<style>
  .mail-card{display:grid;gap:13px;padding:16px;border:1px solid var(--line);border-radius:9px;background:var(--panel)}
  .card-heading{display:flex;align-items:flex-start;justify-content:space-between;gap:12px}.card-heading h3{margin:0 0 4px;font-size:14px}.card-heading p,.hint,.footnote{margin:0;color:var(--muted);font-size:11.5px;line-height:1.5}
  .connected-count{flex:none;padding:3px 8px;border:1px solid var(--line);border-radius:99px;color:var(--muted);font-size:10px}
  button{min-height:31px;padding:6px 10px;border:1px solid var(--line);border-radius:6px;color:var(--ink);background:var(--paper);font:inherit;font-size:11.5px;cursor:pointer}button:disabled{opacity:.55;cursor:not-allowed}.primary{justify-self:start;border-color:var(--accent);color:var(--on-accent,#fff);background:var(--accent)}.refresh-settings{justify-self:start;min-height:25px;padding:3px 7px;font-size:10px}
  .account{display:grid;gap:11px;padding:12px 0;border-top:1px solid var(--line)}.account header{display:flex;align-items:center;justify-content:space-between;gap:10px}.account header div{display:grid;gap:3px;min-width:0}.account strong{overflow:hidden;text-overflow:ellipsis}.account small{color:var(--muted);font-size:10.5px}.disconnect{color:#b84c44}
  fieldset{display:flex;flex-wrap:wrap;gap:8px 16px;margin:0;padding:9px;border:1px solid var(--line);border-radius:6px}legend{padding:0 4px;color:var(--muted);font-size:10.5px}fieldset label{display:flex;align-items:center;gap:6px;font-size:11px}input[type=checkbox]{accent-color:var(--accent)}
  .setup{display:grid;gap:10px;padding-top:11px;border-top:1px solid var(--line)}.setup summary{cursor:pointer;color:var(--muted);font-size:11.5px}.setup[open] summary{margin-bottom:1px}.setup label{display:grid;gap:5px;color:var(--muted);font-size:11px}.setup small{font-size:10px}.setup input{width:100%;box-sizing:border-box;padding:8px 9px;border:1px solid var(--line);border-radius:6px;color:var(--ink);background:var(--paper);font:12px var(--interface-font,sans-serif)}.actions{display:flex;justify-content:flex-end;gap:7px}.pending{display:flex;align-items:center;justify-content:space-between;gap:12px;padding:10px;border:1px solid var(--line);border-radius:7px;font-size:11.5px}.notice{margin:0;color:var(--accent-ink,var(--accent));font-size:11px;line-height:1.45}.error{margin:0;padding:8px 10px;border:1px solid #b84c44;border-radius:6px;color:#b84c44;font-size:11px}.empty{margin:0;padding:10px 0;color:var(--muted);font-size:11.5px}.footnote{padding-top:2px;font-size:10.5px}
</style>
