<script lang="ts">
  import { onMount } from 'svelte';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import Mail from '@lucide/svelte/icons/mail';
  import { getBridge } from '$lib/bridge';
  import MailTriageBatch from '$lib/components/MailTriageBatch.svelte';
  import type { MailAccount, MailBatch } from '$lib/types';

  let { taskId, agentId, inboxes }: { taskId: string; agentId: string; inboxes: MailBatch[] } = $props();
  const bridge = getBridge();
  let accounts = $state<MailAccount[]>([]);
  let selectedAccountId = $state('');
  let query = $state('in:inbox');
  let loading = $state(true);
  let refreshing = $state(false);
  let error = $state('');
  let accountError = $state('');
  let receipt = $state('');

  const selectedAccount = $derived(accounts.find(account => account.id === selectedAccountId) ?? null);
  const accountInboxes = $derived(inboxes
    .filter(batch => batch.nativeAccountId === selectedAccountId)
    .sort((a, b) => (b.updatedAt || b.createdAt) - (a.updatedAt || a.createdAt)));

  async function loadAccounts() {
    loading = true;
    accountError = '';
    try {
      const config = await bridge.getMailConfig();
      accounts = config.accounts.filter(account => account.provider === 'gmail' && account.agentIds.includes(agentId));
      if (!accounts.some(account => account.id === selectedAccountId)) selectedAccountId = accounts[0]?.id ?? '';
    } catch (reason) {
      accountError = String(reason);
    } finally {
      loading = false;
    }
  }

  async function refresh() {
    if (!selectedAccountId || refreshing) return;
    refreshing = true;
    error = '';
    receipt = '';
    try {
      await bridge.refreshMailInbox(taskId, selectedAccountId, query.trim() || 'in:inbox');
      receipt = 'Gmail inbox refreshed.';
    } catch (reason) {
      error = String(reason);
    } finally {
      refreshing = false;
    }
  }

  onMount(() => { void loadAccounts(); });
</script>

<section class="native-mail-inbox" aria-label="Native Gmail inbox">
  <header class="native-mail-controls">
    <span class="native-mail-title"><Mail size={16}/><strong>Gmail inbox</strong></span>
    {#if accounts.length}
      <label>Account
        <select bind:value={selectedAccountId} aria-label="Gmail account">
          {#each accounts as account (account.id)}<option value={account.id}>{account.email}</option>{/each}
        </select>
      </label>
      <label class="mail-query">Search
        <input bind:value={query} aria-label="Gmail search query" placeholder="in:inbox" />
      </label>
      <small class="native-mail-limit">Checks up to 20 Gmail messages; shows only those Jev marks as work requiring your action.</small>
      <button type="button" class="mail-refresh" onclick={() => void refresh()} disabled={refreshing || loading || !selectedAccount}>
        <span class="refresh-icon" class:spin={refreshing}><RefreshCw size={14}/></span>{refreshing ? 'Refreshing…' : 'Refresh'}
      </button>
    {/if}
  </header>
  {#if loading}<p class="native-mail-state" role="status">Loading connected Gmail accounts…</p>
  {:else if accountError}<p class="native-mail-state error" role="alert">{accountError}</p>
  {:else if !accounts.length}<div class="native-mail-state"><strong>No Gmail account is assigned to this agent.</strong><span>Connect a Gmail account in Settings and assign it to this agent to use its native inbox.</span></div>
  {:else}
    {#if error}<p class="native-mail-state error" role="alert">{error}</p>{/if}
    {#if receipt}<p class="native-mail-receipt" role="status">{receipt}</p>{/if}
    {#if accountInboxes.length}
      {#each accountInboxes as inbox (inbox.id)}<MailTriageBatch batch={inbox} {taskId} fullHeight={accountInboxes.length === 1}/>{/each}
    {:else}
      <div class="native-mail-state empty" role="status"><strong>No messages loaded for {selectedAccount?.email}.</strong><span>Refresh this inbox to read messages directly from the selected Gmail account.</span></div>
    {/if}
  {/if}
</section>

<style>
  .native-mail-inbox { display:flex; min-width:0; min-height:0; flex:1; flex-direction:column; gap:10px; }
  .native-mail-controls { display:flex; align-items:end; flex-wrap:wrap; gap:9px; padding:10px 12px; border:1px solid var(--line); border-radius:9px; background:var(--panel); }
  .native-mail-title { display:flex; align-items:center; gap:7px; min-height:31px; margin-right:auto; color:var(--ink); }
  .native-mail-controls label { display:flex; flex-direction:column; gap:4px; color:var(--muted); font:10px var(--mono); }
  .native-mail-limit { align-self:center; color:var(--muted); font-size:10px; }
  .native-mail-controls select,.native-mail-controls input { min-width:180px; height:30px; padding:0 8px; border:1px solid var(--line); border-radius:6px; color:var(--ink); background:var(--paper); font:12px var(--interface-font,system-ui); }
  .native-mail-controls .mail-query input { min-width:240px; }
  .mail-refresh { display:inline-flex; align-items:center; justify-content:center; gap:6px; min-height:31px; padding:0 10px; border:1px solid var(--line); border-radius:6px; color:var(--ink); background:var(--soft); font:11px var(--mono); }
  .mail-refresh:hover:not(:disabled) { border-color:var(--accent); }
  .mail-refresh:disabled { opacity:.55; }
  .refresh-icon { display:inline-flex; }
  .native-mail-state { display:flex; flex-direction:column; gap:5px; padding:18px; border:1px dashed var(--line); border-radius:9px; color:var(--muted); background:var(--panel); font-size:12px; }
  .native-mail-state strong { color:var(--ink); }
  .native-mail-state.error { color:#bd655b; }
  .native-mail-receipt { margin:0; color:var(--muted); font:10px var(--mono); }
  .spin { animation:spin 1s linear infinite; }
  @keyframes spin { to { transform:rotate(360deg); } }
  @media (max-width:700px) { .native-mail-title { width:100%; } .native-mail-controls label,.native-mail-controls select,.native-mail-controls input,.native-mail-controls .mail-query input { flex:1; min-width:0; } }
</style>
