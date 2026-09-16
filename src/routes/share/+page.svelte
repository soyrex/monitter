<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { appThemes } from '$lib/app-theme';
  import type { Message, Snapshot, Task } from '$lib/types';
  import { splitOperatorMessage } from '$lib/operator-sharing';
  import { createMobileSession } from '$lib/controller/remote-client';
  import { redeemPairingCode } from '$lib/controller/pairing-code';

  type VisitorSession = Awaited<ReturnType<typeof createMobileSession>>;
  type DisplayMessage = { id?: string; name: string; text: string; timestamp: number; delivery?: 'sending' | 'sent' | 'uncertain' };
  type LocalOutgoing = DisplayMessage & { id: string; taskId: string; baselineIds: Set<string> };
  let invite = $state(''), code = $state(''), name = $state(''), status = $state('disconnected');
  let verification = $state(''), error = $state(''), connecting = $state(false), sending = $state(false);
  let session = $state<VisitorSession | null>(null), snapshot = $state<Snapshot | null>(null);
  let selectedId = $state<string | null>(null), draft = $state(''), messagesPane = $state<HTMLElement>();
  let localOutgoing = $state<LocalOutgoing[]>([]);
  let unlisten: (() => void) | undefined, generation = 0, refreshing = false;
  const validName = $derived(/^[\p{L}\p{N}][\p{L}\p{N} ._'’-]{1,47}$/u.test(name.trim()));
  const validCode = $derived(/^\d{9}$/.test(code.replace(/[\s-]/g, '')));
  const task = $derived(snapshot?.tasks.find(item => item.id === selectedId) ?? null);
  const messages = $derived(snapshot?.messages.filter(item => item.taskId === selectedId) ?? []);
  const visibleMessages = $derived([
    ...messages.map(message => display(message)),
    ...localOutgoing.filter(item => item.taskId === selectedId),
  ].sort((a, b) => a.timestamp - b.timestamp));
  const projects = $derived(snapshot?.projects ?? []);
  const otherChats = $derived((snapshot?.tasks ?? []).filter(item => !item.projectId));
  const initials = (value: string) => (value.trim().split(/\s+/).filter(Boolean).slice(0, 2).map(word => word[0]).join('') || '?').toUpperCase();
  function isSnapshot(value: unknown): value is Snapshot { return !!value && typeof value === 'object' && Array.isArray((value as Snapshot).tasks) && Array.isArray((value as Snapshot).agents); }
  function display(message: Message): DisplayMessage {
    const tagged = message.role === 'user' ? splitOperatorMessage(message.text) : { name: null, text: message.text };
    const agent = message.senderAgentId ? snapshot?.agents.find(item => item.id === message.senderAgentId) : task?.agentId ? snapshot?.agents.find(item => item.id === task.agentId) : null;
    const name = tagged.name || (message.role === 'assistant' ? agent?.name || 'Agent' : message.role === 'system' ? 'Monitter' : 'Shared participant');
    return { id: message.id, name, text: tagged.text, timestamp: message.createdAt };
  }
  function formatTime(value: number) { return new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' }).format(value); }
  function scrollLatest() { if (messagesPane) messagesPane.scrollTop = messagesPane.scrollHeight; }
  function hexToRgb(hex: string) {
    return [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16));
  }
  function luminance(channels: number[]) {
    return channels.map(c => {
      const v = c / 255;
      return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
    }).reduce((sum, v, i) => sum + v * [0.2126, 0.7152, 0.0722][i], 0);
  }
  function contrast(a: number[], b: number[]) {
    const x = luminance(a), y = luminance(b);
    return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
  }
  function readable(colour: number[], background: number[], target: number) {
    for (let step = 0; step <= 100; step++) {
      const adjusted = colour.map(c => Math.round(c + (target - c) * step / 100));
      if (contrast(adjusted, background) >= 4.5) return adjusted;
    }
    return [target, target, target];
  }
  function select(item: Task) { selectedId = item.id; void tick().then(scrollLatest); }
  function reconcile(next: Snapshot) {
    const consumed = new Set<string>();
    localOutgoing = localOutgoing.filter(item => {
      const match = next.messages.find(message => {
        if (consumed.has(message.id) || item.baselineIds.has(message.id) || message.taskId !== item.taskId || message.role !== 'user') return false;
        const parsed = splitOperatorMessage(message.text);
        return parsed.text === item.text && (parsed.name ?? '') === item.name;
      });
      if (match) { consumed.add(match.id); return false; }
      return true;
    });
  }
  async function refresh() {
    if (!session || refreshing || status !== 'connected') return;
    const current = session; refreshing = true;
    try {
      const next = await current.getSnapshot();
      if (session !== current) return;
      snapshot = next;
      reconcile(next);
      if (selectedId && !next.tasks.some(item => item.id === selectedId)) selectedId = null;
      if (!selectedId && next.tasks.length === 1) selectedId = next.tasks[0].id;
      error = ''; await tick();
    } catch (reason) { if (session === current) error = String(reason); }
    finally { refreshing = false; }
  }
  async function connect() {
    if (connecting || !validName || (!invite && !validCode)) return;
    const currentGeneration = ++generation;
    connecting = true; error = ''; verification = ''; snapshot = null; selectedId = null;
    unlisten?.(); session?.close(); session = null; status = 'connecting';
    try {
      const invitation = invite || await redeemPairingCode(code);
      if (currentGeneration !== generation) return;
      const next = await createMobileSession(invitation, { name: name.trim(), role: 'visitor' });
      if (currentGeneration !== generation) return next.close();
      session = next;
      unlisten = next.subscribe(value => {
        if (currentGeneration !== generation) return;
        status = value.status; verification = value.verificationCode ?? '';
        if (value.error) error = value.error;
        if (value.status === 'connected') void refresh();
      });
    } catch (reason) { if (currentGeneration === generation) { status = 'error'; error = String(reason); } }
    finally { if (currentGeneration === generation) connecting = false; }
  }
  function leave() { generation++; unlisten?.(); unlisten = undefined; session?.close(); session = null; snapshot = null; selectedId = null; draft = ''; localOutgoing = []; verification = ''; status = 'disconnected'; }
  async function send() {
    if (!session || !selectedId || !draft.trim() || sending || status !== 'connected') return;
    const current = session, taskId = selectedId, text = draft.trim(), localId = `local-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    const baselineIds = new Set((snapshot?.messages ?? []).filter(message => message.taskId === taskId).map(message => message.id));
    sending = true;
    localOutgoing = [...localOutgoing, { id: localId, taskId, name: name.trim(), text, timestamp: Date.now(), delivery: 'sending', baselineIds }];
    if (draft.trim() === text) draft = '';
    try {
      const next = await current.sendMessage(taskId, text);
      if (session !== current) return;
      localOutgoing = localOutgoing.map(item => item.id === localId ? { ...item, delivery: 'sent' } : item);
      if (isSnapshot(next)) { snapshot = next; reconcile(next); } else void refresh();
      await tick();
    } catch (reason) {
      if (session === current) { localOutgoing = localOutgoing.map(item => item.id === localId ? { ...item, delivery: 'uncertain' } : item); error = `Message not confirmed: ${String(reason)}`; }
    }
    finally { sending = false; }
  }
  onMount(() => {
    const url = new URL(window.location.href);
    const fragment = new URLSearchParams(url.hash.replace(/^#/, ''));
    let rawFragmentInvite = '';
    if (!url.hash.includes('=')) { try { rawFragmentInvite = decodeURIComponent(url.hash.slice(1)); } catch { rawFragmentInvite = ''; } }
    invite = fragment.get('invite') || rawFragmentInvite || new URLSearchParams(url.search).get('invite') || '';
    const accentParam = fragment.get('accent') || '';
    const themeParam = fragment.get('theme') || '';
    const isDark = themeParam === 'dark' || (themeParam === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
    const palette = appThemes.find(item => item.id === 'monitter')![isDark ? 'dark' : 'light'];
    const root = document.documentElement;
    root.style.colorScheme = isDark ? 'dark' : 'light';
    const colours = {
      '--bg': palette.sidebar, '--surface': palette.paper, '--surface-raised': palette.panel,
      '--border': palette.line, '--text': palette.ink, '--text-muted': palette.muted, '--text-faint': palette.muted,
    };
    for (const [property, value] of Object.entries(colours)) root.style.setProperty(property, value);
    // Keep the shared accent, with readable text for the selected Monitter palette.
    const hex = /^#[0-9a-f]{6}$/i.test(accentParam) ? accentParam.toLowerCase() : palette.accent;
    const colour = hexToRgb(hex);
    const ink = readable(colour, hexToRgb(palette.paper), isDark ? 255 : 0);
    const white = [255, 255, 255], black = [0, 0, 0];
    root.style.setProperty('--accent', hex);
    root.style.setProperty('--accent-rgb', colour.join(', '));
    root.style.setProperty('--accent-ink', `rgb(${ink.join(', ')})`);
    root.style.setProperty('--accent-dark-ink', `rgb(${ink.join(', ')})`);
    root.style.setProperty('--on-accent', contrast(colour, white) >= contrast(colour, black) ? '#ffffff' : '#000000');
    if (invite) { url.searchParams.delete('invite'); url.hash = ''; window.history.replaceState({}, '', url); }
    const timer = window.setInterval(() => { if (!document.hidden) void refresh(); }, 2500);
    return () => { window.clearInterval(timer); generation++; unlisten?.(); session?.close(); };
  });
</script>

<svelte:head><title>Join Monitter</title><meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover" /></svelte:head>

<div class="share-stage">
<main class="share">
  <header><a href="/share" aria-label="Monitter shared workspace">monitter</a>{#if session}<button class="quiet" onclick={leave}>Leave</button>{/if}</header>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if !snapshot}
    <section class="join" aria-labelledby="join-title"><p class="eyebrow">SHARED WORKSPACE</p><h1 id="join-title">Join the conversation</h1>
      {#if status === 'awaiting_approval'}<p>Ask the host to approve your request. Compare this verification code together first.</p><output class="verification" aria-label="Verification code">{verification || '…'}</output><button class="secondary" onclick={leave}>Start again</button>
      {:else if status === 'connecting' || status === 'waiting_for_peer' || connecting}<p>Connecting to the host. Keep this page open while the encrypted connection is established.</p><p class="loading" aria-live="polite">Connecting…</p><button class="secondary" onclick={leave}>Cancel</button>
      {:else}<p>Use your name so people in this shared workspace know who is speaking.</p><label>Your display name<input bind:value={name} minlength="2" maxlength="48" autocomplete="name" placeholder="Alex" aria-describedby="name-help" /></label><small id="name-help">2–48 characters. This is visible with your messages.</small>
        {#if invite}<p class="notice">This collaboration link is ready to join.</p>{:else}<label>Nine-digit collaboration code<input bind:value={code} inputmode="numeric" autocomplete="one-time-code" maxlength="11" placeholder="123 456 789" /></label>{/if}
        <button class="primary" disabled={!validName || (!invite && !validCode) || connecting} onclick={connect}>Request access</button><small>Access is limited to chats and projects the host chooses to share. The link or code works once.</small>{/if}
    </section>
  {:else if !task}
    <section class="workspace" aria-labelledby="workspace-title"><div class="top"><div><p class="eyebrow">CONNECTED</p><h1 id="workspace-title">Shared with you</h1></div><button class="quiet" onclick={refresh}>Refresh</button></div>
      {#each projects as project (project.id)}{@const chats = snapshot.tasks.filter(item => item.projectId === project.id).sort((a,b) => b.updatedAt-a.updatedAt)}<section class="project"><h2>{project.name}</h2>{#if project.description}<p>{project.description}</p>{/if}{#each chats as item (item.id)}<button class="chat" onclick={() => select(item)}><span class="avatar">{initials(item.title)}</span><span><b>{item.title || 'Untitled chat'}</b><small>Shared chat</small></span><span>›</span></button>{:else}<p class="empty">No chats are currently shared in this project.</p>{/each}</section>{/each}
      {#if otherChats.length}<section class="project"><h2>Other shared chats</h2>{#each otherChats.sort((a,b) => b.updatedAt-a.updatedAt) as item (item.id)}<button class="chat" onclick={() => select(item)}><span class="avatar">{initials(item.title)}</span><span><b>{item.title || 'Untitled chat'}</b><small>Shared chat</small></span><span>›</span></button>{/each}</section>{/if}
      {#if !projects.length && !otherChats.length}<p class="empty">The host has not shared any chats yet.</p>{/if}
    </section>
  {:else}
    <section class="conversation" aria-label={task.title || 'Shared chat'}><div class="top"><button class="back" onclick={() => selectedId = null} aria-label="Back to shared chats">‹</button><div><p class="eyebrow">SHARED CHAT</p><h1>{task.title || 'Untitled chat'}</h1></div></div><div class="messages" bind:this={messagesPane} aria-live="polite">{#each visibleMessages as item (item.id ?? `${item.timestamp}-${item.text}`)}<article class:mine={item.name === name.trim()}><span class="avatar">{initials(item.name)}</span><div><header><b>{item.name}</b><time>{formatTime(item.timestamp)}</time></header><p>{item.text}</p>{#if item.delivery}<small class:uncertain={item.delivery === 'uncertain'}>{item.delivery === 'sending' ? 'Sending…' : item.delivery === 'sent' ? 'Sent' : 'Not confirmed'}</small>{/if}</div></article>{:else}<p class="empty">No messages are shared in this chat.</p>{/each}</div><form onsubmit={event => { event.preventDefault(); void send(); }}><label class="sr" for="message">Message</label><textarea id="message" bind:value={draft} rows="2" maxlength="32000" placeholder="Write a message…"></textarea><button class="send" disabled={!draft.trim() || sending || status !== 'connected'}>{sending ? 'Sending…' : 'Send'}</button></form></section>
  {/if}
</main>
</div>

<style>
  :global(*){box-sizing:border-box}
  :global(body){margin:0;background:var(--bg,#e9e3d8);color:var(--text,#262119)}
  .share-stage{height:100dvh;display:grid;place-items:center;padding:24px;overflow:hidden}
  .share{width:100%;max-width:620px;height:min(800px,100%);min-height:0;background:var(--surface,#fbf8f2);border:1px solid var(--border,#dce1d8);border-radius:12px;box-shadow:0 16px 56px #00000020,0 2px 8px #0000000a;font:400 14px/1.5 'IBM Plex Sans',system-ui,sans-serif;display:flex;flex-direction:column;overflow:hidden}
  .share>header{flex:none;min-height:46px;background:var(--surface-raised,#f2ede3)}
  header{min-height:56px;display:flex;align-items:center;justify-content:space-between;padding:0 20px;border-bottom:1px solid var(--border,#dce1d8)}
  header a{color:inherit;font-size:16px;font-weight:600;text-decoration:none}
  h1,h2,p{margin:0}
  h1{font-size:clamp(28px,7vw,42px);line-height:1.08;letter-spacing:-1.3px}
  h2{font-size:19px}
  .eyebrow{color:var(--accent-ink,#45775c);font-size:11px;font-weight:600;letter-spacing:1.2px;margin-bottom:8px}
  .join,.workspace,.conversation{width:100%;flex:1;min-height:0;padding:28px 20px 24px;overflow:auto}
  .conversation{display:flex;flex-direction:column;padding:0;overflow:hidden}
  .join{max-width:460px;margin:auto;display:flex;flex-direction:column;gap:17px;padding:32px 24px}
  .join>p,.project>p,.empty{color:var(--text-muted,#57625b)}
  label{display:grid;gap:7px;color:var(--text,#344239);font-weight:500;font-size:14px}
  input,textarea{width:100%;border:1px solid var(--border,#bfc9be);border-radius:6px;padding:11px 12px;background:var(--surface,#fff);color:inherit;font:inherit}
  input:focus,textarea:focus,button:focus-visible{outline:3px solid var(--accent-dark-ink,#8ac7a4);outline-offset:2px}
  small{color:var(--text-faint,#667169);font-size:12px}
  button{font:inherit;cursor:pointer}
  button:disabled{cursor:not-allowed;opacity:.5}
  .primary,.secondary,.send{min-height:40px;border-radius:6px;padding:10px 16px;font-weight:600}
  .primary,.send{border:0;background:var(--accent,#367a52);color:var(--on-accent,#fff)}
  .secondary{border:1px solid var(--border,#9eaba1);background:transparent;color:inherit}
  .quiet,.back{border:0;background:transparent;color:var(--text-muted,#6f6656);padding:8px}
  .notice,.verification{padding:12px 14px;border-radius:10px;background:color-mix(in srgb, var(--accent) 15%, transparent);color:var(--accent-ink)!important}
  .verification{display:block;padding:20px;font:600 38px/1 'IBM Plex Mono',monospace;letter-spacing:6px;text-align:center}
  .loading{color:var(--accent,#367a52)}
  .error{flex:none;max-height:54px;overflow:auto;overflow-wrap:anywhere;margin:10px 16px 0;padding:10px 12px;border-radius:10px;background:#f8ded9;color:#802b20;font-size:14px}
  .top{display:flex;align-items:center;justify-content:space-between;padding:0 20px;min-height:56px;border-bottom:1px solid var(--border,#dce1d8)}
  .top h1{font-size:16px;line-height:1.35;letter-spacing:-.2px;overflow-wrap:anywhere}
  .top>div{min-width:0}
  .top .eyebrow{font:500 9px 'IBM Plex Mono',monospace;margin-bottom:3px;letter-spacing:.08em;color:var(--text-faint,#667169)}
  .project{margin:24px 0;padding-top:16px;border-top:1px solid var(--border,#dce1d8)}
  .project h2{margin-bottom:6px}
  .chat{width:100%;display:flex;align-items:center;gap:12px;padding:12px 14px;border:1px solid var(--border,#dce1d8);border-radius:12px;background:var(--surface-raised,#f2ede3);cursor:pointer;text-align:left;color:inherit;margin-top:8px}
  .chat:hover{border-color:var(--accent,#367a52)}
  .chat b,.chat small{display:block}
  .chat small{color:var(--text-faint,#667169)}
  .chat span:last-child{margin-left:auto;color:var(--text-faint,#667169);font-size:18px}
  .avatar{width:36px;height:36px;border-radius:8px;background:var(--accent,#367a52);color:var(--on-accent,#fff);display:flex;align-items:center;justify-content:center;font:600 13px 'IBM Plex Mono',monospace;flex:none}
  /* Conversation: fixed topbar, scrollable messages, pinned input */
  .conversation .top{flex:none;justify-content:flex-start;gap:10px;padding:12px 16px}
  .back{flex:none;font-size:22px;line-height:1}
  .conversation .messages{flex:1;min-height:500px;max-height:80vh;overflow-y:auto;overscroll-behavior:contain;padding:24px;display:flex;flex-direction:column;gap:24px}
  .conversation article{display:flex;gap:10px;min-width:0;width:100%;flex:none}
  .conversation article .avatar{width:28px;height:28px;font-size:11px;border-radius:6px}
  .conversation article div{min-width:0;flex:1;display:flex;flex-direction:column;gap:4px}
  .conversation article header{display:flex;align-items:baseline;justify-content:flex-start;flex-wrap:wrap;gap:8px;border:0;min-height:auto;padding:0}
  .conversation article header b{font-size:13px;font-weight:600}
  .conversation article header time{font:400 11px 'IBM Plex Mono',monospace;color:var(--text-faint,#6c6c74)}
  .conversation article p{font-size:14px;line-height:1.65;white-space:pre-wrap;overflow-wrap:anywhere}
  .conversation article.mine .avatar{background:var(--surface-raised,#f2ede3);color:var(--text-muted,#6f6656);border:1px solid var(--border,#dce1d8)}
  .conversation article small{font-size:11px;color:var(--text-faint,#6c6c74)}
  .conversation article small.uncertain{color:#b87a3a}
  .conversation form{flex:none;display:flex;gap:10px;padding:16px;border-top:1px solid var(--border,#dce1d8);background:var(--surface,#fbf8f2);align-items:end}
  .conversation form textarea{flex:1;min-width:0;min-height:64px;max-height:120px;resize:none;border-radius:6px;padding:10px 14px}
  .conversation form .send{min-height:36px;border-radius:6px;padding:0 20px}
  @media(max-width:640px){
    .share-stage{padding:12px}
    .share{height:100%;border-radius:10px;padding-top:env(safe-area-inset-top);padding-bottom:env(safe-area-inset-bottom)}
    input,.conversation form textarea{font-size:16px}
    .conversation .messages{padding:20px 16px}
  }
  @media(max-height:850px){.conversation .messages{min-height:0}}
  @media(max-height:500px){.share-stage{padding:8px}.join{padding:20px}.conversation .top{padding:8px 16px}}
  .sr{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0}
</style>
