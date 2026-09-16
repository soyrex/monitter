<script lang="ts">
  import { activeModal } from "$lib/active-modal";
  import { onDestroy, untrack } from 'svelte';
  import { Check, Copy, Link2, X } from '@lucide/svelte';
  import QRCode from 'qrcode';
  import { getBridge } from '$lib/bridge';
  import { createDesktopSession } from '$lib/controller/remote-client';
  import { DEFAULT_RELAY, registerPairingCode, revokePairingCode, type PairingRegistration } from '$lib/controller/pairing-code';
  import { activeOperatorShare, createOperatorScopedBridge, sharedTaskIds, type ActiveOperatorShare } from '$lib/operator-sharing';
  import type { Snapshot, Task } from '$lib/types';

  let { open = $bindable(false), taskId = null }: { open?: boolean; taskId?: string | null } = $props();
  let relay = $state(DEFAULT_RELAY), primaryName = $state('');
  // This is deliberately a hosted URL, never the desktop WebView origin.
  let publicShareBase = $state(import.meta.env.VITE_MONITTER_PUBLIC_SHARE_URL ?? 'https://share.monitter.com/share');
  let session = $state<Awaited<ReturnType<typeof createDesktopSession>> | null>(null);
  let registration = $state<PairingRegistration | null>(null), status = $state('off'), error = $state('');
  let qr = $state(''), link = $state(''), verificationCode = $state(''), now = $state(Date.now());
  let snapshot = $state<Snapshot | null>(null), taskIds = $state<string[]>([]), projectIds = $state<string[]>([]);
  // An existing encrypted connection never changes scope just because another
  // chat opens its sharing panel. Only a new session can select a new chat.
  let lockedTaskId = $state<string | null>(null);
  let sessionShare = $state<ActiveOperatorShare | null>(null);
  let unsubscribe: (() => void) | undefined, generation = 0, creating = $state(false), profileLoad = 0;
  const timer = setInterval(() => now = Date.now(), 1000);
  const key = $derived(registration && registration.expiresAt > now ? registration.code.match(/.{3}/g)?.join(' ') : '');
  const visitor = $derived(session?.getPeer()?.name ?? 'Visitor');
  const internalAgentIds = $derived(snapshot ? new Set(snapshot.agents.filter(agent => agent.internal === true).map(agent => agent.id)) : new Set<string>());
  const shareableSnapshotTasks = $derived(snapshot ? snapshot.tasks.filter(task => !task.archived && !task.channelId && !internalAgentIds.has(task.agentId)) : []);
  const effectiveTaskIds = $derived(lockedTaskId ? [lockedTaskId] : taskIds.filter(id => shareableSnapshotTasks.some(task => task.id === id)));
  const effectiveProjectIds = $derived(lockedTaskId ? [] : projectIds);
  const selectedTaskIds = $derived(snapshot ? [...sharedTaskIds(snapshot, { taskIds: effectiveTaskIds, projectIds: effectiveProjectIds })] : []);
  const lockedTask = $derived(shareableSnapshotTasks.find(item => item.id === lockedTaskId) ?? null);
  const scopedRequest = $derived(lockedTaskId ?? taskId);
  function clearRegistration() { const current = registration; registration = null; if (current) void revokePairingCode(relay, current).catch(() => {}); }
  async function refresh() { if (!session) return; try { snapshot = await getBridge().getSnapshot(); } catch (reason) { error = String(reason); } }
  function shareableTask(value: Task | null | undefined): value is Task {
    return !!value && !value.archived && !value.channelId;
  }
  function captureSessionShare(): ActiveOperatorShare {
    const primary = Object.freeze({ name: primaryName.trim(), role: 'primary user' as const });
    const peer = Object.freeze({ name: visitor, role: 'visitor' as const });
    const tasks = Object.freeze([...effectiveTaskIds]) as unknown as string[];
    const projects = Object.freeze([...effectiveProjectIds]) as unknown as string[];
    return sessionShare = Object.freeze({ primary, visitor: peer, taskIds: tasks, projectIds: projects });
  }
  function syncActiveShare() {
    const current = untrack(() => sessionShare);
    if (session && status === 'connected' && session.getPeer() && current) activeOperatorShare.set(current);
    else activeOperatorShare.set(null);
  }
  async function loadProfile() {
    const current = ++profileLoad;
    if (!getBridge().available || primaryName.trim()) return;
    try {
      const next = await getBridge().getSnapshot();
      if (current !== profileLoad || session || primaryName.trim()) return;
      snapshot = next;
      primaryName = next.settings.userName?.trim() ?? '';
    } catch { /* Opening the panel remains possible; start shows the error. */ }
  }
  function publicShareUrl(): string | null {
    const value = publicShareBase.trim();
    if (!value) return null;
    try {
      const url = new URL(value);
      if (url.protocol !== 'https:' || url.pathname !== '/share' || url.search || url.hash) return null;
      return url.toString().replace(/\/$/, '');
    } catch { return null; }
  }
  async function start() {
    if (creating) return;
    stop(); error = '';
    const current = ++generation;
    creating = true;
    if (!getBridge().available) { error = 'Open sharing inside Monitter desktop.'; creating = false; return; }
    const publicUrl = publicShareUrl();
    if (!publicUrl) { error = 'Enter the hosted HTTPS /share URL before creating a visitor link.'; creating = false; return; }
    try {
      snapshot = await getBridge().getSnapshot();
      if (current !== generation) return;
      if (!primaryName.trim()) primaryName = snapshot.settings.userName?.trim() ?? '';
    } catch (reason) { error = String(reason); creating = false; return; }
    if (taskId) {
      const selected = snapshot.tasks.find(item => item.id === taskId);
      const ownerAgent = selected ? snapshot.agents.find(agent => agent.id === selected.agentId) : null;
      if (!shareableTask(selected) || ownerAgent?.internal === true) {
        error = 'This chat can no longer be shared. Choose an active direct chat and try again.';
        creating = false;
        return;
      }
      // Capture before the invitation/session exists: a visitor link from this
      // action is permanently limited to this one task.
      lockedTaskId = selected.id;
      taskIds = [selected.id]; projectIds = [];
    }
    if (!/^[\p{L}\p{N}][\p{L}\p{N} ._'’-]{1,47}$/u.test(primaryName.trim())) { error = 'Enter your profile name (2–48 letters, numbers, spaces or punctuation) before sharing.'; creating = false; return; }
    try {
      let candidate: typeof session = null;
      const next = await createDesktopSession(relay, createOperatorScopedBridge(getBridge(), () =>
        generation === current && session === candidate && status === 'connected' ? sessionShare : null));
      candidate = next;
      if (current !== generation) { next.close(); return; }
      session = next;
      unsubscribe = next.subscribe(state => {
        status = state.status; verificationCode = state.verificationCode ?? '';
        if (state.status === 'pending' || state.status === 'connected') captureSessionShare();
        syncActiveShare();
        if (state.error) error = state.error;
        if (['pending', 'connected', 'closed', 'error', 'rejected'].includes(state.status)) clearRegistration();
      });
      // The invite is a fragment so it is never sent to the hosted web server
      // or leaked through its request logs/referrers.
      link = `${publicUrl}#invite=${encodeURIComponent(next.invitation)}&accent=${encodeURIComponent(snapshot.settings.accent ?? '#3f9d6a')}&theme=${snapshot.settings.theme ?? 'light'}`;
      qr = await QRCode.toDataURL(link, { width: 260, margin: 2 });
      if (current !== generation) { next.close(); return; }
      const created = await registerPairingCode(next.invitation);
      if (current !== generation || !['connecting', 'waiting_for_peer'].includes(next.getStatus())) { void revokePairingCode(relay, created).catch(() => {}); return; }
      registration = created;
      await refresh();
    } catch (reason) { if (current === generation) error = String(reason); }
    finally { if (current === generation) creating = false; }
  }
  async function approve() {
    if (primaryName.trim().toLocaleLowerCase() === visitor.trim().toLocaleLowerCase()) {
      error = 'Use distinct names for the owner and visitor so model attribution remains unambiguous.';
      return;
    }
    try { await session?.approve(); captureSessionShare(); syncActiveShare(); await refresh(); } catch (reason) { error = String(reason); }
  }
  async function copyLink() { try { await navigator.clipboard.writeText(link); } catch { error = 'Copy the share link from the browser address field instead.'; } }
  function stop() {
    generation++; creating = false; profileLoad++; unsubscribe?.(); unsubscribe = undefined; session?.close(); session = null; clearRegistration();
    qr = ''; link = ''; status = 'off'; verificationCode = ''; taskIds = []; projectIds = []; lockedTaskId = null; sessionShare = null; activeOperatorShare.set(null);
  }
  function toggle(list: string[], id: string) { return list.includes(id) ? list.filter(item => item !== id) : [...list, id]; }
  $effect(() => {
    const current = session, state = status; primaryName; visitor; effectiveTaskIds; effectiveProjectIds;
    // Workspace sharing remains editable after approval. A task invite never
    // enters this branch, so its captured task-only scope remains immutable.
    if (current && state === 'connected' && current.getPeer() && !lockedTaskId) captureSessionShare();
    syncActiveShare();
  });
  $effect(() => { if (open && !session) void loadProfile(); });
  $effect(() => { if (status === 'connected') { const interval = setInterval(() => void refresh(), 2_000); return () => clearInterval(interval); } });
  onDestroy(() => { clearInterval(timer); stop(); });
</script>

{#if open}<dialog use:activeModal class="share-panel" open aria-label={scopedRequest ? 'Share this chat' : 'Share workspace'}>
  <header><div><strong>{scopedRequest ? 'Share this chat' : 'Share with a collaborator'}</strong><small>{scopedRequest ? 'One visitor, one exact chat, one-use encrypted link.' : 'One visitor, one-use encrypted link.'}</small></div><button aria-label="Close sharing" onclick={() => open = false}><X size={18}/></button></header>
  {#if !session}
    {#if taskId}<p class="scope-note">This invite will be limited to the selected direct chat. It cannot include its project or any other chat.</p>{/if}
    <label>Your name<input aria-label="Primary operator name" bind:value={primaryName} maxlength="48" placeholder="Your profile name"/><small>Defaults to your saved profile name.</small></label>
    <label>Relay address<input aria-label="Share relay address" bind:value={relay}/></label>
    <label>Hosted share URL<input aria-label="Hosted share URL" bind:value={publicShareBase} inputmode="url"/><small>The visitor opens this HTTPS page; the encrypted invite stays in the link fragment.</small></label>
    <button class="primary" disabled={creating} onclick={start}><Link2 size={15}/>{creating ? 'Creating secure link…' : 'Create one-use share link'}</button>
  {:else}
    <p class="status" role="status">{status.replaceAll('_', ' ')}</p>
    {#if qr && ['connecting', 'waiting_for_peer'].includes(status)}
      <img src={qr} alt="One-use sharing QR code"/>
      {#if key}<div class="quote-code"><small>Quote this code to your collaborator</small><strong>{key}</strong><small>Expires in {Math.max(0, Math.ceil(((registration?.expiresAt ?? 0) - now) / 1000))} seconds and works once.</small></div>{/if}
      <button class="secondary" onclick={copyLink}><Copy size={14}/>Copy share link</button>
    {:else if status === 'pending'}
      <p><b>{visitor}</b> wants to join{lockedTask ? ` this chat, “${lockedTask.title || 'Untitled chat'}”` : ''}. Compare this code together before approving.</p><strong class="verification">{verificationCode}</strong>
      <button class="primary" onclick={approve}><Check size={15}/>Approve {visitor}</button><button class="secondary" onclick={stop}>Reject</button>
    {:else if status === 'connected'}
      {#if lockedTask}<p><b>{visitor}</b> is paired with <b>{lockedTask.title || 'Untitled chat'}</b>. Their access is locked to this chat.</p>{#if taskId && taskId !== lockedTaskId}<p class="scope-note">Only one visitor session can be active. End this share before creating a link for another chat.</p>{/if}
      {:else}<p><b>{visitor}</b> is paired. Choose exactly what they can see and message. Nothing is shared until selected.</p>
        <div class="share-list"><h3>Chats</h3>{#each shareableSnapshotTasks as task}<label class="scope"><input type="checkbox" checked={taskIds.includes(task.id)} onchange={() => taskIds = toggle(taskIds, task.id)}/><span><b>{task.title || 'Untitled chat'}</b><small>{snapshot?.agents.find(agent => agent.id === task.agentId)?.name ?? 'Agent'}</small></span></label>{:else}<small>No shareable chats yet.</small>{/each}</div>
        <div class="share-list"><h3>Projects</h3>{#each snapshot?.projects ?? [] as project}<label class="scope"><input type="checkbox" checked={projectIds.includes(project.id)} onchange={() => projectIds = toggle(projectIds, project.id)}/><span><b>{project.name}</b><small>Shares its current chats</small></span></label>{:else}<small>No projects yet.</small>{/each}</div>
      {/if}
      <small>{selectedTaskIds.length} chat{selectedTaskIds.length === 1 ? '' : 's'} shared. The visitor can send messages only; they cannot stop agents, use terminals, or view local paths, attachments, instructions, hosts or activity.</small>
    {:else if ['closed', 'error', 'rejected'].includes(status)}<button class="primary" onclick={start}>Create a fresh share link</button>{/if}
    <button class="danger" onclick={stop}>End sharing and revoke access</button>
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</dialog>{/if}

<style>
  .share-panel{position:fixed;right:18px;bottom:18px;z-index:100;width:min(390px,calc(100vw - 36px));max-height:85vh;margin:0;overflow:auto;padding:18px;border:1px solid var(--line);border-radius:14px;background:var(--panel);color:var(--ink);box-shadow:0 12px 40px #0006;font:13px var(--interface-font,"IBM Plex Sans",sans-serif)}header{display:flex;justify-content:space-between;gap:10px}header strong,header small{display:block}header small,.share-panel small{color:var(--muted)}button{border:0;border-radius:7px;padding:9px 10px;background:var(--soft);color:var(--ink);cursor:pointer}button.primary{background:var(--accent);color:var(--on-accent)}button.secondary{border:1px solid var(--line)}button.danger{color:#b84c44;margin-top:10px}.share-panel>label{display:grid;gap:5px;margin:13px 0;color:var(--muted)}input{width:100%;padding:8px;border:1px solid var(--line);border-radius:6px;background:var(--paper);color:var(--ink);font:inherit}.status{text-transform:capitalize;color:var(--accent-ink)}img{display:block;max-width:250px;margin:12px auto;border-radius:8px}.quote-code{display:grid;gap:5px;text-align:center;margin:12px 0}.quote-code strong,.verification{font:600 25px var(--mono);letter-spacing:3px;color:var(--accent-ink)}.verification{display:block;text-align:center;margin:16px}.share-list{margin:15px 0;padding-top:10px;border-top:1px solid var(--line)}.share-list h3{margin:0 0 8px;font-size:12px;text-transform:uppercase;letter-spacing:.08em;color:var(--muted)}.scope{display:flex;gap:9px;align-items:start;padding:7px 0}.scope input{width:auto;margin-top:3px;accent-color:var(--accent)}.scope b,.scope small{display:block}.error{color:#b84c44;line-height:1.4}
</style>
