<script lang="ts">
  import { onDestroy } from 'svelte';
  import { Check, Copy, Link2, LoaderCircle, UserRound, X } from '@lucide/svelte';
  import QRCode from 'qrcode';
  import { getBridge } from '$lib/bridge';
  import { createDesktopSession } from '$lib/controller/remote-client';
  import { DEFAULT_RELAY, registerPairingCode, revokePairingCode, type PairingRegistration } from '$lib/controller/pairing-code';
  import { activeOperatorShare, formatOperatorMessage, sharedSnapshot, sharedTaskIds, type ActiveOperatorShare } from '$lib/operator-sharing';
  import type { Snapshot, TerminalRead, TerminalSession } from '$lib/types';

  let { open = $bindable(false) }: { open?: boolean } = $props();
  let relay = $state(DEFAULT_RELAY), primaryName = $state('Alex');
  let session = $state<Awaited<ReturnType<typeof createDesktopSession>> | null>(null);
  let registration = $state<PairingRegistration | null>(null), status = $state('off'), error = $state('');
  let qr = $state(''), link = $state(''), verificationCode = $state(''), now = $state(Date.now());
  let snapshot = $state<Snapshot | null>(null), taskIds = $state<string[]>([]), projectIds = $state<string[]>([]);
  let unsubscribe: (() => void) | undefined, generation = 0;
  const timer = setInterval(() => now = Date.now(), 1000);
  const key = $derived(registration && registration.expiresAt > now ? registration.code.match(/.{3}/g)?.join(' ') : '');
  const visitor = $derived(session?.getPeer()?.name ?? 'Visitor');
  const selectedTaskIds = $derived(snapshot ? [...sharedTaskIds(snapshot, { taskIds, projectIds })] : []);

  function share(): ActiveOperatorShare {
    return { primary: { name: primaryName.trim(), role: 'primary user' }, visitor: { name: visitor, role: 'visitor' }, taskIds, projectIds };
  }
  function clearRegistration() { const current = registration; registration = null; if (current) void revokePairingCode(relay, current).catch(() => {}); }
  async function refresh() { if (!session) return; try { snapshot = await getBridge().getSnapshot(); } catch (reason) { error = String(reason); } }
  function scopedBridge() {
    const bridge = getBridge();
    const allowed = async (taskId: string) => {
      const current = await bridge.getSnapshot();
      if (!sharedTaskIds(current, share()).has(taskId)) throw new Error('This chat is not shared with this collaborator.');
      return current;
    };
    return {
      getSnapshot: async () => sharedSnapshot(await bridge.getSnapshot(), share()),
      sendMessage: async (taskId: string, text: string) => {
        await allowed(taskId);
        const result = await bridge.sendMessage(taskId, formatOperatorMessage([share().primary, share().visitor], share().visitor, text));
        // Remote-controller protocol requires a snapshot. A desktop fast ack
        // deliberately does not contain one, so refresh only at this boundary.
        const next = 'tasks' in result ? result : await bridge.getSnapshot();
        return sharedSnapshot(next, share());
      },
      cancelTask: async () => { throw new Error('Collaborators cannot stop a shared agent.'); },
      resumeTask: async () => { throw new Error('Collaborators cannot resume a shared agent.'); },
      listTerminals: async (): Promise<TerminalSession[]> => [],
      readTerminal: async (): Promise<TerminalRead> => { throw new Error('Terminals are not shared.'); },
    };
  }
  async function start() {
    stop(); error = '';
    if (!/^[\p{L}\p{N}][\p{L}\p{N} ._'’-]{1,47}$/u.test(primaryName.trim())) { error = 'Use a primary name of 2–48 letters, numbers, spaces or punctuation.'; return; }
    if (!getBridge().available) { error = 'Open sharing inside Monitter desktop.'; return; }
    const current = ++generation;
    try {
      const next = await createDesktopSession(relay, scopedBridge());
      if (current !== generation) { next.close(); return; }
      session = next;
      unsubscribe = next.subscribe(state => {
        status = state.status; verificationCode = state.verificationCode ?? '';
        if (state.error) error = state.error;
        if (['pending', 'connected', 'closed', 'error', 'rejected'].includes(state.status)) clearRegistration();
      });
      qr = await QRCode.toDataURL(next.invitation, { width: 260, margin: 2 });
      link = `${window.location.origin}/share?invite=${encodeURIComponent(next.invitation)}`;
      const created = await registerPairingCode(next.invitation);
      if (current !== generation || !['connecting', 'waiting_for_peer'].includes(next.getStatus())) { void revokePairingCode(relay, created).catch(() => {}); return; }
      registration = created;
      await refresh();
    } catch (reason) { if (current === generation) error = String(reason); }
  }
  async function approve() { try { await session?.approve(); await refresh(); } catch (reason) { error = String(reason); } }
  async function copyLink() { try { await navigator.clipboard.writeText(link); } catch { error = 'Copy the share link from the browser address field instead.'; } }
  function stop() {
    generation++; unsubscribe?.(); unsubscribe = undefined; session?.close(); session = null; clearRegistration();
    qr = ''; link = ''; status = 'off'; verificationCode = ''; taskIds = []; projectIds = []; activeOperatorShare.set(null);
  }
  function toggle(list: string[], id: string) { return list.includes(id) ? list.filter(item => item !== id) : [...list, id]; }
  $effect(() => { const current = session; const state = status; primaryName; visitor; taskIds; projectIds; if (current && state === 'connected' && current.getPeer()) activeOperatorShare.set({ ...share(), taskIds: selectedTaskIds, projectIds: [] }); else activeOperatorShare.set(null); });
  $effect(() => { if (status === 'connected') { const interval = setInterval(() => void refresh(), 2_000); return () => clearInterval(interval); } });
  onDestroy(() => { clearInterval(timer); stop(); });
</script>

{#if open}<dialog class="share-panel" open aria-label="Share workspace">
  <header><div><strong>Share with a collaborator</strong><small>One visitor, one-use encrypted link.</small></div><button aria-label="Close sharing" onclick={() => open = false}><X size={18}/></button></header>
  {#if !session}
    <label>Your name<input aria-label="Primary operator name" bind:value={primaryName} maxlength="48"/></label>
    <label>Relay address<input aria-label="Share relay address" bind:value={relay}/></label>
    <button class="primary" onclick={start}><Link2 size={15}/>Create one-use share link</button>
  {:else}
    <p class="status" role="status">{status.replaceAll('_', ' ')}</p>
    {#if qr && ['connecting', 'waiting_for_peer'].includes(status)}
      <img src={qr} alt="One-use sharing QR code"/>
      {#if key}<div class="quote-code"><small>Quote this code to your collaborator</small><strong>{key}</strong><small>Expires in {Math.max(0, Math.ceil(((registration?.expiresAt ?? 0) - now) / 1000))} seconds and works once.</small></div>{/if}
      <button class="secondary" onclick={copyLink}><Copy size={14}/>Copy share link</button>
    {:else if status === 'pending'}
      <p><b>{visitor}</b> wants to join. Compare this code together before approving.</p><strong class="verification">{verificationCode}</strong>
      <button class="primary" onclick={approve}><Check size={15}/>Approve {visitor}</button><button class="secondary" onclick={stop}>Reject</button>
    {:else if status === 'connected'}
      <p><b>{visitor}</b> is paired. Choose exactly what they can see and message. Nothing is shared until selected.</p>
      <div class="share-list"><h3>Chats</h3>{#each snapshot?.tasks.filter(task => !task.archived && !task.channelId) ?? [] as task}<label class="scope"><input type="checkbox" checked={taskIds.includes(task.id)} onchange={() => taskIds = toggle(taskIds, task.id)}/><span><b>{task.title || 'Untitled chat'}</b><small>{snapshot?.agents.find(agent => agent.id === task.agentId)?.name ?? 'Agent'}</small></span></label>{:else}<small>No shareable chats yet.</small>{/each}</div>
      <div class="share-list"><h3>Projects</h3>{#each snapshot?.projects ?? [] as project}<label class="scope"><input type="checkbox" checked={projectIds.includes(project.id)} onchange={() => projectIds = toggle(projectIds, project.id)}/><span><b>{project.name}</b><small>Shares its current chats</small></span></label>{:else}<small>No projects yet.</small>{/each}</div>
      <small>{selectedTaskIds.length} chat{selectedTaskIds.length === 1 ? '' : 's'} shared. The visitor can send messages only; they cannot stop agents, use terminals, or view local paths, attachments, instructions, hosts or activity.</small>
    {:else if ['closed', 'error', 'rejected'].includes(status)}<button class="primary" onclick={start}>Create a fresh share link</button>{/if}
    <button class="danger" onclick={stop}>End sharing and revoke access</button>
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</dialog>{/if}

<style>
  .share-panel{position:fixed;right:18px;bottom:18px;z-index:100;width:min(390px,calc(100vw - 36px));max-height:85vh;margin:0;overflow:auto;padding:18px;border:1px solid var(--line);border-radius:14px;background:var(--panel);color:var(--ink);box-shadow:0 12px 40px #0006;font:13px var(--interface-font,"IBM Plex Sans",sans-serif)}header{display:flex;justify-content:space-between;gap:10px}header strong,header small{display:block}header small,.share-panel small{color:var(--muted)}button{border:0;border-radius:7px;padding:9px 10px;background:var(--soft);color:var(--ink);cursor:pointer}button.primary{background:var(--accent);color:var(--on-accent)}button.secondary{border:1px solid var(--line)}button.danger{color:#b84c44;margin-top:10px}.share-panel>label{display:grid;gap:5px;margin:13px 0;color:var(--muted)}input{width:100%;padding:8px;border:1px solid var(--line);border-radius:6px;background:var(--paper);color:var(--ink);font:inherit}.status{text-transform:capitalize;color:var(--accent-ink)}img{display:block;max-width:250px;margin:12px auto;border-radius:8px}.quote-code{display:grid;gap:5px;text-align:center;margin:12px 0}.quote-code strong,.verification{font:600 25px var(--mono);letter-spacing:3px;color:var(--accent-ink)}.verification{display:block;text-align:center;margin:16px}.share-list{margin:15px 0;padding-top:10px;border-top:1px solid var(--line)}.share-list h3{margin:0 0 8px;font-size:12px;text-transform:uppercase;letter-spacing:.08em;color:var(--muted)}.scope{display:flex;gap:9px;align-items:start;padding:7px 0}.scope input{width:auto;margin-top:3px;accent-color:var(--accent)}.scope b,.scope small{display:block}.error{color:#b84c44;line-height:1.4}
</style>
