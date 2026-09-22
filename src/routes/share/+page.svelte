<script lang="ts">
import { onMount, tick } from 'svelte';
import { Paperclip, ArrowUp, Shield, Brain, Zap, X, PanelLeft, Info, LoaderCircle, Image } from '@lucide/svelte';
import ContextUsageBar from '$lib/components/ContextUsageBar.svelte';
import type { ComposerContextUsage } from '$lib/context-usage-data';
import ImageLightbox from '$lib/components/ImageLightbox.svelte';
import Markdown from '$lib/components/Markdown.svelte';
import MessageMeta from '$lib/components/MessageMeta.svelte';
import ResponseMetadata from '$lib/components/ResponseMetadata.svelte';
import ProviderIcon from '$lib/components/ProviderIcon.svelte';
import type { Attachment, AttachmentFileData, Message, Snapshot, Task } from '$lib/types';
import { splitOperatorMessage } from '$lib/operator-sharing';
import { participantColour, applySharedAppearance, type SharedChatSnapshot, type SharedChatAppearance } from '$lib/shared-chat';
import { appThemes } from '$lib/app-theme';
import { contrastForeground } from '$lib/accent-contrast';
import { createMobileSession, type CollaboratorMobileSession } from '$lib/controller/remote-client';
import { redeemPairingCode } from '$lib/controller/pairing-code';
type VisitorSession = CollaboratorMobileSession;
type DisplayMessage = {
    id?: string;
    name: string;
    text: string;
    timestamp: number;
    attachments: Attachment[];
    role: Message['role'];
    streamStatus?: Message['streamStatus'];
    phase?: Message['phase'];
    responseMetadata?: Message['responseMetadata'];
    delivery?: 'sending' | 'sent' | 'uncertain';
};
type LocalOutgoing = DisplayMessage & {
    id: string;
    taskId: string;
    baselineIds: Set<string>;
};
let invite = $state(''), code = $state(''), name = $state(''), status = $state('disconnected'), verification = $state(''), error = $state(''), connecting = $state(false), sending = $state(false), uploading = $state(false);
let session = $state<VisitorSession | null>(null), snapshot = $state<SharedChatSnapshot | null>(null), selectedId = $state<string | null>(null), draft = $state(''), messagesPane = $state<HTMLElement>(), picker = $state<HTMLInputElement>();
let localOutgoing = $state<LocalOutgoing[]>([]), attachments = $state<Attachment[]>([]), sidebarOpen = $state(false), detailsOpen = $state(false), following = true;
let lightbox = $state<{
    src: string;
    alt: string;
    title?: string;
} | null>(null);
let refreshError = $state('');
const chatDrafts = new Map<string, {
    text: string;
    files: Attachment[];
}>();
let unlisten: (() => void) | undefined, generation = 0, refreshing = false;
const validName = $derived(/^[\p{L}\p{N}][\p{L}\p{N} ._'’-]{1,47}$/u.test(name.trim()));
const validCode = $derived(/^\d{9}$/.test(code.replace(/[\s-]/g, '')));
const task = $derived(snapshot?.tasks.find(x => x.id === selectedId) ?? null);
const messages = $derived(snapshot?.messages.filter(x => x.taskId === selectedId) ?? []);
const uploadLimits = $derived(snapshot?.sharing?.uploads ?? { maxFileBytes: 0, maxFiles: 0 });
const composerInfo = $derived(selectedId ? snapshot?.sharing?.composerByTask?.[selectedId] : undefined);
const modelLabel = $derived(composerInfo?.model.label || composerInfo?.model.id || task?.modelSettings?.model || task?.model || 'Harness default');
const effortLabel = $derived(composerInfo?.reasoningEffort || task?.modelSettings?.reasoningEffort || 'Harness default');
const fastMode = $derived(composerInfo?.fastMode ?? task?.modelSettings?.fastMode ?? false);
const permissionLabel = $derived(task ? ({'read-only':'Read only','workspace-write':'Workspace write','harness-configured':'Harness permissions','yolo':'YOLO'}[task.sandbox] ?? task.sandbox) : 'Unavailable');
const contextUsage = $derived<ComposerContextUsage>(composerInfo?.context ?? {status:'unavailable',reason:'Context usage is unavailable until the host reports its context window.'});
const contextLabel = $derived(contextUsage.status === 'available' ? `Context ${Math.round(contextUsage.usedPercent)}%` : 'Context unavailable');
function composerKeydown(event: KeyboardEvent) {
    if (event.key !== 'Enter' || event.isComposing || event.keyCode === 229 || event.shiftKey) return;
    event.preventDefault();
    void send();
}
const looseTasks = $derived(snapshot?.tasks.filter(x => !x.projectId) ?? []);
const visibleMessages = $derived([...messages.map(display), ...localOutgoing.filter(x => x.taskId === selectedId)].sort((a, b) => a.timestamp - b.timestamp));
const initials = (value: string) => (value.trim().split(/\s+/).filter(Boolean).slice(0, 2).map(w => w[0]).join('') || '?').toUpperCase();
function isSnapshot(x: unknown): x is SharedChatSnapshot { return !!x && typeof x === 'object' && Array.isArray((x as Snapshot).tasks) && Array.isArray((x as Snapshot).agents); }
function display(message: Message): DisplayMessage { const tagged = message.role === 'user' ? splitOperatorMessage(message.text) : { name: null, text: message.text }; const agent = message.senderAgentId ? snapshot?.agents.find(x => x.id === message.senderAgentId) : task?.agentId ? snapshot?.agents.find(x => x.id === task.agentId) : null; return { id: message.id, name: tagged.name || (message.role === 'assistant' ? agent?.name || 'Agent' : message.role === 'system' ? 'Monitter' : snapshot?.sharing?.primary.name || 'Shared participant'), text: tagged.text, timestamp: message.createdAt, attachments: message.attachments ?? [], role: message.role, streamStatus: message.streamStatus, phase: message.phase, responseMetadata: message.responseMetadata }; }
function isHuman(item: DisplayMessage) { return item.role === 'user'; }
function scrollLatest() { if (messagesPane)
    messagesPane.scrollTop = messagesPane.scrollHeight; }
function trackScroll() { if (messagesPane)
    following = messagesPane.scrollHeight - messagesPane.scrollTop - messagesPane.clientHeight < 32; }
function select(item: Task) {
    if (selectedId)
        chatDrafts.set(selectedId, { text: draft, files: attachments });
    selectedId = item.id;
    const saved = chatDrafts.get(item.id);
    draft = saved?.text ?? '';
    attachments = saved?.files ?? [];
    sidebarOpen = false;
    detailsOpen = false;
    following = true;
    void tick().then(scrollLatest);
}
function reconcile(next: SharedChatSnapshot) {
    const used = new Set<string>();
    localOutgoing = localOutgoing.filter(item => {
        const found = next.messages.find(message => {
            if (used.has(message.id) || item.baselineIds.has(message.id) || message.taskId !== item.taskId || message.role !== 'user') return false;
            const parsed = splitOperatorMessage(message.text);
            // The authenticated host is authoritative. Older hosts saved the
            // entered visitor as "Visitor"; after a successful send, consume
            // that confirmed echo rather than leaving a second "Sent" bubble.
            const approvedVisitor = next.sharing?.visitor.name ?? 'Visitor';
            const sameAuthor = parsed.name === item.name || (item.delivery === 'sent' && parsed.name === approvedVisitor);
            const attachmentIds = (message.attachments ?? []).map(file => file.id).sort();
            const sameAttachments = JSON.stringify(attachmentIds) === JSON.stringify(item.attachments.map(file => file.id).sort());
            return sameAuthor && parsed.text === item.text && sameAttachments;
        });
        if (!found) return true;
        used.add(found.id);
        return false;
    });
}
function apply(next: SharedChatSnapshot) { snapshot = next; applySharedAppearance(next.sharing?.appearance); reconcile(next); if (selectedId && !next.tasks.some(x => x.id === selectedId)) {
    selectedId = null;
    attachments = [];
} if (!selectedId && next.tasks.length)
    select(next.tasks[0]); }
async function refresh() { if (!session || refreshing || status !== 'connected')
    return; const current = session; refreshing = true; try {
    const next = await current.getSnapshot();
    if (session !== current)
        return;
    apply(next as SharedChatSnapshot);
    refreshError = '';
    await tick();
    if (following)
        scrollLatest();
}
catch (reason) {
    if (session === current)
        refreshError = `Connection refresh failed: ${String(reason)}`;
}
finally {
    refreshing = false;
} }
async function connect() { if (connecting || !validName || (!invite && !validCode))
    return; const current = ++generation; connecting = true; error = ''; verification = ''; snapshot = null; selectedId = null; unlisten?.(); session?.close(); session = null; status = 'connecting'; try {
    const token = invite || await redeemPairingCode(code);
    if (current !== generation)
        return;
    const next = await createMobileSession(token, { name: name.trim(), role: 'visitor' });
    if (current !== generation)
        return next.close();
    session = next;
    unlisten = next.subscribe(value => { if (current !== generation)
        return; status = value.status; verification = value.verificationCode ?? ''; if (value.error)
        error = value.error; if (value.status === 'connected')
        void refresh(); });
}
catch (reason) {
    if (current === generation) {
        status = 'error';
        error = String(reason);
    }
}
finally {
    if (current === generation)
        connecting = false;
} }
function leave() { generation++; unlisten?.(); unlisten = undefined; session?.close(); session = null; snapshot = null; selectedId = null; draft = ''; attachments = []; localOutgoing = []; verification = ''; status = 'disconnected'; uploading = false; sending = false; error = ''; refreshError = ''; chatDrafts.clear(); }
async function fileData(file: File): Promise<AttachmentFileData> { const bytes = new Uint8Array(await file.arrayBuffer()); let binary = ''; for (let i = 0; i < bytes.length; i += 0x8000)
    binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000)); return { filename: file.name || 'attachment', mimeType: file.type || 'application/octet-stream', dataBase64: btoa(binary) }; }
async function thumbnail(file: File): Promise<string | undefined> {
    if (!file.type.startsWith('image/'))
        return undefined;
    try {
        const source = await createImageBitmap(file), scale = Math.min(1, 320 / Math.max(source.width, source.height));
        const canvas = document.createElement('canvas');
        canvas.width = Math.max(1, Math.round(source.width * scale));
        canvas.height = Math.max(1, Math.round(source.height * scale));
        canvas.getContext('2d')?.drawImage(source, 0, 0, canvas.width, canvas.height);
        source.close();
        const preview = canvas.toDataURL('image/jpeg', .7);
        return preview.length <= 48000 ? preview : undefined;
    }
    catch {
        return undefined;
    }
}
async function attachFiles(files: File[]) {
    if (!session || !selectedId || uploading || sending || !uploadLimits.maxFiles || status !== 'connected')
        return;
    const current = session, taskId = selectedId, currentGeneration = generation;
    const failures: string[] = [];
    const room = uploadLimits.maxFiles - attachments.length;
    if (files.length > room)
        failures.push(`Only ${room} more files can be attached.`);
    files = files.slice(0, Math.max(0, room));
    if (!files.length) {
        error = failures.join(' ');
        return;
    }
    uploading = true;
    try {
        for (const file of files) {
            if (session !== current || generation !== currentGeneration)
                break;
            try {
                if (file.size > uploadLimits.maxFileBytes)
                    throw new Error(`${file.name} exceeds the ${Math.floor(uploadLimits.maxFileBytes / 1024 / 1024)} MB limit.`);
                const data = await fileData(file), previewDataUrl = await thumbnail(file);
                if (session !== current || generation !== currentGeneration)
                    break;
                const stored = await current.storeAttachment(taskId, { ...data, previewDataUrl });
                if (session !== current || generation !== currentGeneration)
                    break;
                if (selectedId === taskId)
                    attachments = [...attachments, stored];
                else {
                    const saved = chatDrafts.get(taskId) ?? { text: '', files: [] };
                    chatDrafts.set(taskId, { ...saved, files: [...saved.files, stored] });
                }
            }
            catch (reason) {
                failures.push(String(reason));
            }
        }
        if (session === current)
            error = failures.length ? `Attachment was not added: ${failures.join(' ')}` : '';
    }
    finally {
        if (session === current && generation === currentGeneration)
            uploading = false;
    }
}
function dropFiles(event: DragEvent) { event.preventDefault(); void attachFiles(Array.from(event.dataTransfer?.files ?? [])); }
function pasteFiles(event: ClipboardEvent) { const files = Array.from(event.clipboardData?.files ?? []); if (files.length) {
    if (!event.clipboardData?.getData('text/plain')) event.preventDefault();
    void attachFiles(files);
} }
async function send() { if (!session || !selectedId || (!draft.trim() && !attachments.length) || sending || uploading || status !== 'connected')
    return; const current = session, taskId = selectedId, text = draft, sentAttachments = [...attachments], localId = `local-${Date.now()}-${Math.random().toString(36).slice(2)}`, baselineIds = new Set((snapshot?.messages ?? []).filter(x => x.taskId === taskId).map(x => x.id)); sending = true; localOutgoing = [...localOutgoing, { id: localId, taskId, name: name.trim(), text, timestamp: Date.now(), attachments: sentAttachments, role: 'user', delivery: 'sending', baselineIds }]; if (draft === text)
    draft = ''; attachments = []; try {
    const next = await current.sendMessage(taskId, text, sentAttachments.map(x => x.id));
    if (session !== current)
        return;
    localOutgoing = localOutgoing.map(x => x.id === localId ? { ...x, delivery: 'sent' } : x);
    if (isSnapshot(next))
        apply(next);
    else
        void refresh();
    await tick();
    if (following)
        scrollLatest();
}
catch (reason) {
    if (session === current) {
        localOutgoing = localOutgoing.map(x => x.id === localId ? { ...x, delivery: 'uncertain' } : x);
        error = `Message not confirmed: ${String(reason)}`;
    }
}
finally {
    if (session === current)
        sending = false;
} }
onMount(() => {
    const url = new URL(window.location.href);
    const fragment = new URLSearchParams(url.hash.replace(/^#/, ''));
    let rawInvite = '';
    if (!url.hash.includes('=')) {
        try {
            rawInvite = decodeURIComponent(url.hash.slice(1));
        }
        catch { }
    }
    invite = fragment.get('invite') || rawInvite || url.searchParams.get('invite') || '';
    const theme = fragment.get('theme') === 'dark' ? 'dark' : 'light';
    const palette = appThemes.find(item => item.id === 'monitter')?.[theme];
    if (palette) {
        const root = document.documentElement;
        root.style.colorScheme = theme;
        root.dataset.theme = theme;
        root.style.setProperty('--accent-ink', palette.ink);
        root.style.setProperty('--on-accent', contrastForeground(fragment.get('accent') || palette.accent));
        for (const [token, value] of Object.entries({ '--paper': palette.paper, '--sidebar': palette.sidebar, '--panel': palette.panel, '--line': palette.line, '--soft': palette.soft, '--code': palette.code, '--ink': palette.ink, '--muted': palette.muted, '--accent': /^#[0-9a-f]{6}$/i.test(fragment.get('accent') ?? '') ? fragment.get('accent')! : palette.accent }))
            root.style.setProperty(token, value);
    }
    const encoded = fragment.get('appearance');
    if (encoded)
        try {
            applySharedAppearance(JSON.parse(encoded) as SharedChatAppearance);
        }
        catch { }
    if (invite) {
        url.searchParams.delete('invite');
        url.hash = '';
        window.history.replaceState({}, '', url);
    }
    const timer = window.setInterval(() => { if (!document.hidden)
        void refresh(); }, 2500);
    return () => { window.clearInterval(timer); generation++; unlisten?.(); session?.close(); };
});
</script>
<!-- Mobile sidebar starts closed; Escape always dismisses an open drawer. -->
<svelte:window onkeydown={event => { if (event.key === 'Escape') { sidebarOpen = false; detailsOpen = false; } }} />
<svelte:head>
<title>Join Monitter</title>
<meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no, viewport-fit=cover"/>
</svelte:head>
<div class="share-stage">
<main class="share">
<header class="brand">
<a href="/share" aria-label="Monitter shared workspace">monitter</a>
<span>Shared workspace</span>{#if session}<button class="quiet" onclick={leave}>Leave</button>{/if}</header>{#if error || refreshError}<p class="error" role="alert">{error || refreshError}<button class="quiet" aria-label="Dismiss error" onclick={()=>{error='';refreshError='';}}>
<X size={14}/>
</button>
</p>{/if}
{#if !snapshot}<section class="join" aria-labelledby="join-title">
<p class="eyebrow">SHARED WORKSPACE</p>
<h1 id="join-title">Join the conversation</h1>{#if status==='awaiting_approval'}<p>Ask the host to approve your request. Compare this verification code together first.</p>
<output class="verification" aria-label="Verification code">{verification||'…'}</output>
<button class="secondary" onclick={leave}>Start again</button>{:else if status==='connecting'||status==='waiting_for_peer'||connecting}<p>Connecting to the host. Keep this page open while the encrypted connection is established.</p>
<p class="loading">Connecting…</p>
<button class="secondary" onclick={leave}>Cancel</button>{:else}<p>Use your name so people in this shared workspace know who is speaking.</p>
<label>Your display name<input bind:value={name} minlength="2" maxlength="48" autocomplete="name" placeholder="Alex"/>
</label>
<small>2–48 characters. This is visible with your messages.</small>{#if invite}<p class="notice">This collaboration link is ready to join.</p>{:else}<label>Nine-digit collaboration code<input bind:value={code} inputmode="numeric" autocomplete="one-time-code" maxlength="11" placeholder="123 456 789"/>
</label>{/if}<button class="primary" disabled={!validName||(!invite&&!validCode)||connecting} onclick={connect}>Request access</button>
<small>Access is limited to chats and projects the host chooses to share. The link or code works once.</small>{/if}</section>
{:else}<div class:sidebar-open={sidebarOpen} class="shared-shell">{#if sidebarOpen}<button class="nav-backdrop" aria-label="Close shared chats" onclick={()=>sidebarOpen=false}>
</button>{/if}<aside class="shared-nav" aria-label="Shared chats">
<div class="nav-title">
<b>Shared with you</b>
<button class="icon mobile-nav" aria-label="Hide shared chats" onclick={()=>sidebarOpen=false}>
<X size={16}/>
</button>
<button class="quiet" onclick={refresh}>Refresh</button>
</div>{#each snapshot.projects as project (project.id)}{@const chats=snapshot.tasks.filter(x=>x.projectId===project.id)}{#if chats.length}<section>
<small>{project.name}</small>{#each chats as item (item.id)}<button class:active={item.id===selectedId} onclick={()=>select(item)}>{initials(item.title)} <span>{item.title||'Untitled chat'}</span>
</button>{/each}</section>{/if}{/each}{#if looseTasks.length}<section>
<small>OTHER SHARED CHATS</small>{#each looseTasks as item (item.id)}<button class:active={item.id===selectedId} onclick={()=>select(item)}>{initials(item.title)} <span>{item.title||'Untitled chat'}</span>
</button>{/each}</section>{/if}</aside>
{#if task}<section class="conversation" aria-label={task.title||'Shared chat'} ondragover={e=>e.preventDefault()} ondrop={dropFiles} onpaste={pasteFiles}>
<header class="conversation-head">
<button class="icon mobile-nav" aria-label="Show shared chats" aria-expanded={sidebarOpen} onclick={()=>sidebarOpen=!sidebarOpen}>
<PanelLeft size={17}/>
</button>
<div class="heading">
<p class="eyebrow">SHARED CHAT</p>
<h1>{task.title||'Untitled chat'}</h1>
</div>
<button class="icon" aria-label="Chat details" aria-expanded={detailsOpen} onclick={()=>detailsOpen=!detailsOpen}>
<Info size={17}/>
</button>
</header>{#if detailsOpen}<aside class="details" aria-label="Read-only chat details">
<div class="detail-title">
<b>Chat details</b>
<button class="quiet" aria-label="Close chat details" onclick={()=>detailsOpen=false}>
<X size={14}/>
</button>
</div>
<dl>
<dt>Agent</dt>
<dd>{snapshot.agents.find(x=>x.id===task.agentId)?.name||'Agent'}</dd>
<dt>Provider</dt>
<dd>
<ProviderIcon provider={task.provider} size={13}/>{task.provider}</dd>
<dt>Model</dt>
<dd>{modelLabel}</dd>
<dt>Reasoning</dt>
<dd>{effortLabel}</dd>
<dt>Fast mode</dt>
<dd>{task.modelSettings?.fastMode==null?'Default':task.modelSettings.fastMode?'On':'Off'}</dd>
<dt>Sandbox</dt>
<dd>{permissionLabel}</dd>
<dt>Access</dt>
<dd>Shared chat only</dd>
</dl>
</aside>{/if}<div role="log" aria-label="Chat messages" class="messages" bind:this={messagesPane} onscroll={trackScroll} onwheel={e=>{if(e.deltaY<0)following=false}} ontouchmove={()=>following=false} aria-live="polite">{#each visibleMessages as item (item.id??`${item.timestamp}-${item.text}`)}<article class:final-answer={item.phase==='final_answer'} data-stream-status={item.streamStatus} class:user={item.role==='user'} style={isHuman(item)?`--participant:${participantColour(item.name)}`:''}>
<div class="bubble">
<MessageMeta name={item.name} createdAt={item.timestamp}>{#snippet avatar()}<span class="avatar" style={isHuman(item)?`background:${participantColour(item.name)}`:''}>{initials(item.name)}</span>{/snippet}{#if item.delivery}<span class:uncertain={item.delivery==='uncertain'} class="delivery">{item.delivery==='sending'?'Sending…':item.delivery==='sent'?'Sent':'Not confirmed'}</span>{/if}</MessageMeta>
<Markdown text={item.text} preserveLineBreaks={item.role==='user'}/>{#if item.streamStatus==='streaming'}<small class="stream-state" role="status">Receiving…</small>{:else if item.streamStatus==='interrupted'}<small class="stream-state">Response interrupted</small>{/if}{#if item.attachments.length}<div class="attachment-list">{#each item.attachments as attachment (attachment.id)}<div>
<span>▤</span>
<b>{attachment.name}</b>
<small>{Math.max(1,Math.round(attachment.size/1024))} KB</small>{#if attachment.previewDataUrl}<button class="preview-button" aria-label={`Preview ${attachment.name}`} onclick={()=>lightbox={src:attachment.previewDataUrl!,alt:attachment.name,title:attachment.name}}>
<img src={attachment.previewDataUrl} alt={`Preview of ${attachment.name}`}/>
</button>{/if}</div>{/each}</div>{/if}</div>
{#if item.role==='assistant' && item.responseMetadata}<ResponseMetadata metadata={item.responseMetadata}/>{/if}
</article>{:else}<p class="empty">No messages are shared in this chat.</p>{/each}</div>{#if task.status==='running'}<p class="agent-status" role="status">
<LoaderCircle class="spin" size={13}/>Agent is working…</p>{/if}<form onsubmit={e=>{e.preventDefault();void send()}}>
<div class="composer">
  {#if attachments.length}
    <div class="pending" aria-label="Attached files">
      {#each attachments as attachment (attachment.id)}
        <span><Image size={13}/>{attachment.name}<button type="button" aria-label={`Remove attachment ${attachment.name}`} onclick={()=>attachments=attachments.filter(x=>x.id!==attachment.id)}><X size={13}/></button></span>
      {/each}
    </div>
  {/if}
  <textarea aria-label="Message" bind:value={draft} onkeydown={composerKeydown} rows="2" maxlength="32000" placeholder={`Message ${snapshot.agents.find(agent=>agent.id===task.agentId)?.name || 'agent'}…`} aria-describedby="composer-keyboard-hint"></textarea>
  <div class="composer-footer">
    <div class="composer-left">
      <button class="icon attachment-control" type="button" aria-label="Attach files" title={uploadLimits.maxFiles ? `Attach files (up to ${uploadLimits.maxFiles}, ${Math.round(uploadLimits.maxFileBytes/1024/1024)} MB each)` : 'Attachments unavailable'} disabled={!uploadLimits.maxFiles||uploading||sending||status!=='connected'} onclick={()=>picker?.click()}>
        {#if uploading}<LoaderCircle class="spin" size={14}/>{:else}<Paperclip size={14}/>{/if}
      </button>
      <input bind:this={picker} class="file-input" type="file" multiple onchange={e=>{const files=Array.from(e.currentTarget.files??[]);e.currentTarget.value='';void attachFiles(files)}}/>
      <span class="readonly-setting permission-setting" aria-label={`Permissions: ${permissionLabel}`} title="Permissions set by the host (read-only)"><Shield size={14}/><span>{permissionLabel}</span></span>
    </div>
    <div class="composer-right">
      <span class="readonly-setting model-setting" aria-label={`Model: ${modelLabel}. Reasoning effort: ${effortLabel}`} title={`Model: ${modelLabel} · Reasoning effort: ${effortLabel} · Set by the host (read-only)`}>
        <Brain size={14}/>{#if fastMode}<Zap size={11} aria-label="Fast mode on"/>{/if}<span class="model-label">{modelLabel}</span><small class="effort-label">{effortLabel}</small>
      </span>
      <button class="send composer-control" aria-label="Send" title={task.status==='running' ? 'Send follow-up' : 'Send message (Enter)'} disabled={(!draft.trim()&&!attachments.length)||sending||uploading||status!=='connected'}>
        {#if sending}<LoaderCircle class="spin" size={15}/>{:else}<ArrowUp size={16}/>{/if}
      </button>
    </div>
  </div>
  <div class="composer-hints"><span id="composer-keyboard-hint">Enter to send · Shift+Enter for a new line</span><span class="context-label" title={contextUsage.status==='available' ? `${contextUsage.used.toLocaleString()} of ${contextUsage.size.toLocaleString()} tokens used` : contextUsage.reason}>{contextLabel}</span></div>
  <ContextUsageBar usage={contextUsage}/>
</div></form>
</section>{:else}<section class="empty-state">
<h1>Shared with you</h1>
<p>The host has not shared any chats yet.</p>
</section>{/if}</div>{/if}</main>
</div>
<ImageLightbox bind:image={lightbox}/>
<style>
:global(*){box-sizing:border-box}
:global(body){margin:0;background:var(--sidebar,#20211f);color:var(--ink,#e9e7e1)}
.share-stage{height:100dvh;display:grid;place-items:center;padding:24px;overflow:hidden;font:400 14px/1.5 var(--interface-font,'IBM Plex Sans',system-ui,sans-serif)}
.share{width:min(1180px,100%);height:min(800px,100%);min-height:0;display:flex;flex-direction:column;overflow:hidden;border:1px solid var(--line,#3d403b);border-radius:12px;background:var(--paper,#282a27);box-shadow:0 18px 58px #0007}
.brand{flex:none;height:46px;display:flex;gap:12px;align-items:center;padding:0 16px;border-bottom:1px solid var(--line);background:var(--panel,#30322f)}
.brand a{color:var(--ink);font-weight:600;text-decoration:none}
.brand span{color:var(--muted)}
button{font:inherit;color:inherit;cursor:pointer}
button:focus-visible,textarea:focus-visible,input:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
.quiet,.icon{border:0;background:transparent;color:var(--muted);padding:7px}
.brand .quiet{margin-left:auto}
.primary,.secondary,.send{min-height:40px;border-radius:7px;padding:9px 14px;font-weight:600}
.primary,.send{border:0;background:var(--accent);color:var(--on-accent,#fff)}
.secondary{border:1px solid var(--line);background:transparent}
.error{z-index:2;margin:10px 16px 0;padding:9px 12px;border-radius:7px;background:#6d332c;color:#fff;overflow-wrap:anywhere}
.join{width:min(460px,100%);margin:auto;padding:32px 24px;display:flex;flex-direction:column;gap:16px;overflow:auto}
.join h1{font-size:38px;line-height:1.1;margin:0}
.eyebrow{margin:0 0 4px;color:var(--muted);font:500 10px var(--mono,monospace);letter-spacing:.11em}
label{display:grid;gap:7px;font-weight:500}
input,textarea{border:1px solid var(--line);border-radius:7px;padding:10px 12px;background:var(--panel);color:var(--ink);font:inherit}
.notice,.verification{padding:12px;border-radius:7px;background:color-mix(in srgb,var(--accent) 18%,var(--panel));color:var(--accent-ink)}
.verification{text-align:center;font:600 34px var(--mono,monospace);letter-spacing:.15em}
.loading{color:var(--accent)}
small{color:var(--muted);font-size:11px}
.shared-shell{position:relative;flex:1;min-height:0;display:grid;grid-template-columns:244px minmax(0,1fr);overflow:hidden}
.shared-nav{padding:13px 10px;overflow:auto;border-right:1px solid var(--line);background:var(--sidebar)}
.nav-title{display:flex;align-items:center;justify-content:space-between;margin:0 5px 15px}
.shared-nav section{display:grid;gap:3px;margin-bottom:18px}
.shared-nav section>small{padding:0 6px}
.shared-nav section button{display:flex;gap:9px;align-items:center;padding:8px;border:0;border-radius:6px;background:transparent;text-align:left}
.shared-nav section button.active{background:var(--soft);color:var(--ink)}
.shared-nav section button:not(.active){color:var(--muted)}
.shared-nav section button span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.conversation{position:relative;display:flex;min-width:0;min-height:0;flex-direction:column;background:var(--paper)}
.conversation-head{flex:none;display:flex;align-items:center;gap:10px;min-height:62px;padding:10px 16px;border-bottom:1px solid var(--line);background:var(--paper)}
.heading{min-width:0;flex:1}
.heading h1{margin:0;overflow:hidden;font-size:17px;line-height:1.35;text-overflow:ellipsis;white-space:nowrap}
.mobile-nav{display:none}
.details{position:absolute;z-index:3;right:14px;top:54px;width:240px;padding:14px;border:1px solid var(--line);border-radius:8px;background:var(--panel);box-shadow:0 12px 30px #0005}
.details dl{display:grid;grid-template-columns:82px 1fr;gap:7px;margin:12px 0 0}
.details dt{color:var(--muted);font-size:11px}
.details dd{display:flex;gap:5px;align-items:center;min-width:0;margin:0;overflow-wrap:anywhere}
.messages{flex:1;scrollbar-gutter:stable;min-height:500px;max-height:80vh;overflow:auto;overscroll-behavior:contain;padding:28px clamp(18px,4vw,50px)}
article{max-width:100%;margin:0 0 24px}
.bubble{max-width:900px}
.conversation :global(.avatar){display:grid;flex:none;place-items:center;width:21px;height:21px;border-radius:5px;background:var(--soft);color:#fff;font:600 10px var(--mono,monospace)}
article.user .bubble{width:fit-content;min-width:min(260px,100%);margin-left:auto;padding:11px 13px;border-radius:10px 10px 3px 10px;background:color-mix(in srgb,var(--participant) 17%,var(--panel));border:1px solid color-mix(in srgb,var(--participant) 30%,var(--line))}
.delivery{margin-left:auto;color:var(--muted);font:400 10px var(--mono,monospace)}
.delivery.uncertain{color:#e4a35c}
.attachment-list,.pending{display:flex;flex-wrap:wrap;gap:7px;margin-top:9px}
.attachment-list>div,.pending>span{display:flex;gap:6px;align-items:center;max-width:100%;padding:5px 7px;border:1px solid var(--line);border-radius:6px;background:var(--panel);font-size:11px}
.attachment-list b,.pending span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.attachment-list img{display:block;max-width:80px;max-height:54px;border-radius:4px}
.pending{padding:0 16px 11px;margin:0}
.pending>span{max-width:220px}
.pending button{display:grid;padding:0;border:0;background:transparent;color:var(--muted)}
form{flex:none;padding:0 16px 12px;background:var(--paper)}
.composer{position:relative;box-sizing:border-box;margin:0 auto;padding:11px 12px 9px;border:1px solid var(--line);border-radius:10px;background:color-mix(in srgb,var(--panel) 80%,transparent);backdrop-filter:blur(16px);box-shadow:0 8px 30px #0002;overflow:hidden}
.composer textarea{font-family:var(--chat-font,'IBM Plex Sans',system-ui,sans-serif);display:block;width:100%;min-height:52px;max-height:25vh;resize:vertical;border:0;outline:0;padding:2px;color:var(--ink);background:transparent;font-size:var(--chat-font-size,13px);line-height:var(--chat-line-height,1.65)}
.composer textarea:focus-visible{outline:none;box-shadow:none}
.composer textarea::placeholder{color:var(--muted)}
.composer-footer{display:flex;align-items:center;justify-content:space-between;flex-wrap:wrap;gap:6px 10px;color:var(--muted);font:calc(10px * var(--interface-font-ratio,1)) var(--mono,monospace)}
.composer-left,.composer-right{display:flex;align-items:center;gap:6px;min-width:0}
.composer-right{margin-left:auto;max-width:100%}
.readonly-setting{display:flex;align-items:center;gap:6px;min-width:0;padding:5px 3px;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio,1));line-height:1.35}
.readonly-setting :global(svg){flex:none}
.model-label{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.effort-label{flex:none;text-transform:capitalize;color:var(--muted);font:inherit;font-size:10px}
.file-input{display:none}
.send.composer-control{width:30px;min-width:30px;height:30px;min-height:30px;flex:none;padding:0;display:inline-grid;place-items:center;border-radius:50%;background:var(--accent);color:var(--on-accent)}
.attachment-control{padding:5px;display:grid;place-items:center}
.composer-hints{display:flex;justify-content:space-between;gap:8px;margin-top:7px;color:var(--muted);font:9px/1.4 var(--mono,monospace)}
.context-label{white-space:nowrap}
.composer .pending{padding:0 0 8px}
:global(.spin){animation:spin .8s linear infinite}
@keyframes spin{to{transform:rotate(360deg)}
}
.empty,.empty-state{color:var(--muted)}
.empty-state{padding:40px}
.empty-state h1{color:var(--ink)}
@media(max-width:700px){.share-stage{padding:0}
.share{height:100%;border:0;border-radius:0}
.shared-shell{grid-template-columns:1fr}
.shared-nav{position:absolute;z-index:4;inset:0 auto 0 0;width:min(82vw,300px);transform:translateX(-102%);transition:transform .16s ease;box-shadow:12px 0 30px #0005}
.sidebar-open .shared-nav{transform:translateX(0)}
.mobile-nav{display:block}
.messages{padding:20px 16px}
.conversation .messages{min-height:0}
}
@media(max-height:850px){.messages{min-height:0}
}

.conversation :global(.avatar){display:grid;flex:none;place-items:center;width:21px;height:21px;border-radius:5px;background:var(--soft);color:#fff;font:600 10px var(--mono,monospace)}

.messages{font:var(--chat-font-size,13px)/var(--chat-line-height,1.65) var(--chat-font,'IBM Plex Sans',system-ui,sans-serif)}

@media(max-width:700px){.shared-nav{display:none;background:var(--sidebar,#191b19);opacity:1;z-index:8}
.sidebar-open .shared-nav{display:block;transform:translateX(0)}
.shared-shell:not(.sidebar-open) .shared-nav{display:none}
}

.nav-backdrop{display:none}
.detail-title{display:flex;align-items:center;justify-content:space-between}
.details{max-height:calc(100% - 70px);overflow:auto}
.error{display:flex;align-items:center;justify-content:space-between;flex:none}
.error .quiet{color:inherit}
.preview-button{padding:0;border:0;background:transparent}
.agent-status{display:flex;align-items:center;gap:8px;flex:none;margin:0;padding:8px 16px;color:var(--muted);font-size:11px}
.stream-state{display:block;margin-top:8px}
.final-answer .bubble{width:fit-content;padding:12px 14px;border:1px solid color-mix(in srgb,#4f9d69 18%,var(--line));border-radius:10px 10px 10px 3px;background:color-mix(in srgb,#4f9d69 7%,var(--panel))}
button:disabled{opacity:.45;cursor:default}

@media(max-width:700px){.nav-backdrop{display:block;position:absolute;inset:0;z-index:7;border:0;background:#0007}
.shared-nav{z-index:8}
}

</style>
