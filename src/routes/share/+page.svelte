<script lang="ts">
  import { onMount, tick } from 'svelte';
  import type { Message, Snapshot, Task } from '$lib/types';
  import { createMobileSession } from '$lib/controller/remote-client';
  import { redeemPairingCode } from '$lib/controller/pairing-code';

  type VisitorSession = Awaited<ReturnType<typeof createMobileSession>>;
  type DisplayMessage = { name: string; text: string };
  const collaborationHeader = /^\[Two human operators are collaborating[^\n]*\]\s*\n/i;
  let invite = $state(''), code = $state(''), name = $state(''), status = $state('disconnected');
  let verification = $state(''), error = $state(''), connecting = $state(false), sending = $state(false);
  let session = $state<VisitorSession | null>(null), snapshot = $state<Snapshot | null>(null);
  let selectedId = $state<string | null>(null), draft = $state(''), messagesPane = $state<HTMLElement>();
  let unlisten: (() => void) | undefined, generation = 0, refreshing = false;
  const validName = $derived(/^[\p{L}\p{N}][\p{L}\p{N} ._'’-]{1,47}$/u.test(name.trim()));
  const validCode = $derived(/^\d{9}$/.test(code.replace(/[\s-]/g, '')));
  const task = $derived(snapshot?.tasks.find(item => item.id === selectedId) ?? null);
  const messages = $derived(snapshot?.messages.filter(item => item.taskId === selectedId) ?? []);
  const projects = $derived(snapshot?.projects ?? []);
  const otherChats = $derived((snapshot?.tasks ?? []).filter(item => !item.projectId));
  const initials = (value: string) => (value.trim().split(/\s+/).filter(Boolean).slice(0, 2).map(word => word[0]).join('') || '?').toUpperCase();
  function isSnapshot(value: unknown): value is Snapshot { return !!value && typeof value === 'object' && Array.isArray((value as Snapshot).tasks) && Array.isArray((value as Snapshot).agents); }
  function display(message: Message): DisplayMessage {
    const clean = message.text.replace(collaborationHeader, '');
    const tagged = /^@\(([^\r\n)]{1,48})\):\s*([\s\S]*)$/.exec(clean);
    if (tagged?.[1].trim()) return { name: tagged[1].trim(), text: tagged[2] };
    return { name: message.role === 'user' ? 'You' : 'Monitter', text: clean };
  }
  function scrollLatest() { if (messagesPane) messagesPane.scrollTop = messagesPane.scrollHeight; }
  function select(item: Task) { selectedId = item.id; void tick().then(scrollLatest); }
  async function refresh() {
    if (!session || refreshing || status !== 'connected') return;
    const current = session; refreshing = true;
    try {
      const next = await current.getSnapshot();
      if (session !== current) return;
      snapshot = next;
      if (selectedId && !next.tasks.some(item => item.id === selectedId)) selectedId = null;
      error = ''; await tick(); scrollLatest();
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
  function leave() { generation++; unlisten?.(); unlisten = undefined; session?.close(); session = null; snapshot = null; selectedId = null; draft = ''; verification = ''; status = 'disconnected'; }
  async function send() {
    if (!session || !selectedId || !draft.trim() || sending || status !== 'connected') return;
    const current = session, text = draft.trim(); sending = true;
    try { const next = await current.sendMessage(selectedId, text); if (session !== current) return; if (isSnapshot(next)) snapshot = next; else void refresh(); draft = ''; await tick(); scrollLatest(); }
    catch (reason) { if (session === current) error = String(reason); }
    finally { sending = false; }
  }
  onMount(() => {
    invite = new URLSearchParams(window.location.search).get('invite') ?? '';
    const timer = window.setInterval(() => { if (!document.hidden) void refresh(); }, 2500);
    return () => { window.clearInterval(timer); generation++; unlisten?.(); session?.close(); };
  });
</script>

<svelte:head><title>Join Monitter</title><meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover" /></svelte:head>

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
    <section class="conversation" aria-label={task.title || 'Shared chat'}><div class="top"><button class="back" onclick={() => selectedId = null} aria-label="Back to shared chats">‹</button><div><p class="eyebrow">SHARED CHAT</p><h1>{task.title || 'Untitled chat'}</h1></div></div><div class="messages" bind:this={messagesPane} aria-live="polite">{#each messages as message (message.id)}{@const item = display(message)}<article class:mine={item.name === name.trim()}><span class="avatar">{initials(item.name)}</span><div><b>{item.name}</b><p>{item.text}</p></div></article>{:else}<p class="empty">No messages are shared in this chat.</p>{/each}</div><form onsubmit={event => { event.preventDefault(); void send(); }}><label class="sr" for="message">Message</label><textarea id="message" bind:value={draft} rows="2" maxlength="32000" placeholder="Write a message…"></textarea><button class="send" disabled={!draft.trim() || sending || status !== 'connected'}>{sending ? 'Sending…' : 'Send'}</button></form></section>
  {/if}
</main>

<style>
  :global(*){box-sizing:border-box}:global(body){margin:0;background:#f6f7f3;color:#18201c}.share{min-height:100dvh;max-width:760px;margin:auto;padding:env(safe-area-inset-top) 20px env(safe-area-inset-bottom);font:400 16px/1.5 'IBM Plex Sans',system-ui,sans-serif;display:flex;flex-direction:column}header{min-height:72px;display:flex;align-items:center;justify-content:space-between;border-bottom:1px solid #dce1d8}header a{color:inherit;font-size:21px;font-weight:600;text-decoration:none}h1,h2,p{margin:0}h1{font-size:clamp(28px,7vw,42px);line-height:1.08;letter-spacing:-1.3px}h2{font-size:19px}.eyebrow{color:#45775c;font-size:11px;font-weight:600;letter-spacing:1.2px;margin-bottom:8px}.join,.workspace,.conversation{width:100%;flex:1;padding:48px 0 24px}.join{max-width:460px;margin:auto;display:flex;flex-direction:column;justify-content:center;gap:17px}.join>p,.project>p,.empty{color:#57625b}label{display:grid;gap:7px;color:#344239;font-weight:500;font-size:14px}input,textarea{width:100%;border:1px solid #bfc9be;border-radius:12px;padding:13px 14px;background:#fff;color:inherit;font:inherit}input:focus,textarea:focus,button:focus-visible{outline:3px solid #8ac7a4;outline-offset:2px}small{color:#667169;font-size:12px}button{font:inherit;cursor:pointer}button:disabled{cursor:not-allowed;opacity:.5}.primary,.secondary,.send{min-height:48px;border-radius:12px;padding:10px 16px;font-weight:600}.primary,.send{border:0;background:#367a52;color:#fff}.secondary{border:1px solid #9eaba1;background:transparent;color:inherit}.quiet,.back{border:0;background:transparent;color:#275f40;padding:8px}.notice,.verification{padding:12px 14px;border-radius:10px;background:#e4f1e8;color:#275f40!important}.verification{display:block;padding:20px;font:600 38px/1 'IBM Plex Mono',monospace;letter-spacing:6px;text-align:center}.loading{color:#367a52}.error{margin:14px 0 -20px;padding:12px 14px;border-radius:10px;background:#f8ded9;color:#802b20;font-size:14px}.top{display:flex;align-items:center;justify-content:space-between;gap:14px}.project{padding:25px 0;border-bottom:1px solid #dce1d8;display:grid;gap:8px}.chat{width:100%;display:flex;align-items:center;gap:12px;border:0;border-radius:12px;padding:12px 8px;background:transparent;color:inherit;text-align:left}.chat:hover{background:#ebeee9}.chat>span:nth-child(2){flex:1;min-width:0}.chat b{display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.chat small{display:block}.avatar{flex:none;width:34px;height:34px;display:inline-grid;place-items:center;border-radius:50%;background:#d8ebdd;color:#275f40;font-size:12px;font-weight:600}.conversation{display:flex;flex-direction:column;min-height:0;padding-bottom:0}.conversation .top{padding-bottom:20px}.conversation .top>div{flex:1;min-width:0}.conversation h1{font-size:25px;overflow:hidden;white-space:nowrap;text-overflow:ellipsis}.back{font-size:37px;line-height:1}.messages{flex:1;min-height:160px;overflow:auto;padding:8px 0 20px}.messages article{display:flex;align-items:flex-start;gap:10px;margin:14px 0}.messages article>div{max-width:min(85%,590px);padding:10px 13px;border-radius:4px 14px 14px;background:#e7ebe5}.messages article.mine{flex-direction:row-reverse}.messages article.mine>div{background:#d6efdf;border-radius:14px 4px 14px 14px}.messages b{font-size:13px;color:#31523e}.messages article p{white-space:pre-wrap;overflow-wrap:anywhere}.conversation form{display:flex;align-items:flex-end;gap:9px;padding:14px 0 calc(14px + env(safe-area-inset-bottom));border-top:1px solid #dce1d8}.conversation textarea{flex:1;resize:vertical;min-height:48px;max-height:180px}.send{flex:none}.sr{position:absolute;width:1px;height:1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap}@media(max-width:520px){.share{padding-left:16px;padding-right:16px}.join,.workspace,.conversation{padding-top:32px}.send{min-width:72px}}@media(prefers-color-scheme:dark){:global(body){background:#151a16;color:#edf2eb}.share header,.project,.conversation form{border-color:#374239}.join>p,.project>p,.empty,small{color:#abb6ac}label{color:#d4ddd5}input,textarea{background:#202820;border-color:#566358;color:#edf2eb}.notice,.verification{background:#203b29;color:#bce6c9!important}.chat:hover{background:#222b23}.avatar{background:#294434;color:#c2efd0}.messages article>div{background:#28312a}.messages article.mine>div{background:#244633}.messages b{color:#bce6c9}.error{background:#542d29;color:#ffd8d2}}
</style>
