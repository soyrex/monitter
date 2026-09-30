<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import Send from '@lucide/svelte/icons/send';
  import X from '@lucide/svelte/icons/x';
  import ApprovalDock from '$lib/components/ApprovalDock.svelte';
  import Markdown from '$lib/components/Markdown.svelte';
  import { createCompanionVoice, isCompanionAvailable, setCompanionActive } from '$lib/companion-voice';
  import { createCompanionReplyInbox } from '$lib/companion-replies';
  import { readCompanionDrafts, saveCompanionDraft } from '$lib/companion-drafts';
  import { mobileViewport } from '$lib/mobile-viewport';
  import type { Agent, ApprovalDecision, ApprovalRequest, Message, Task } from '$lib/types';

  let { tasks, agents, approvals, messages, initialTaskId, connected, onclose, onresolve, oninput, onsend }: {
    tasks: Task[];
    agents: Agent[];
    approvals: ApprovalRequest[];
    messages: Message[];
    initialTaskId: string | null;
    connected: boolean;
    onclose: () => void;
    onresolve: (request: ApprovalRequest, decision: ApprovalDecision) => Promise<void>;
    oninput: (request: ApprovalRequest, response: unknown) => Promise<void>;
    onsend: (taskId: string, text: string) => Promise<void>;
  } = $props();

  const eligibleTasks = $derived(tasks.filter(task => !task.archived && !task.channelId && !agents.find(agent => agent.id === task.agentId)?.internal));
  function recentTaskId() {
    const preferred = initialTaskId ? eligibleTasks.find(task => task.id === initialTaskId) : undefined;
    return preferred?.id ?? [...eligibleTasks].sort((a, b) => b.updatedAt - a.updatedAt)[0]?.id ?? null;
  }
  let selectedTaskId = $state<string | null>(recentTaskId());
  let previousTaskId = untrack(() => selectedTaskId);
  let draftsByTask = $state<Record<string, string>>(readCompanionDrafts());
  let sending = $state(false);
  let sendError = $state('');
  let deliveryNotice = $state('');
  let approvalError = $state('');
  let resolvingId = $state<string | null>(null);
  let voiceEnabled = $state(false);
  let autoSend = $state(false);
  let editingReview = $state(false);
  let voiceAvailable = $state(false);
  let voiceLevel = $state(0);
  let voiceState = $state({ phase: 'off' as 'off' | 'arming' | 'waiting' | 'listening' | 'finishing' | 'error', transcript: '', error: '' });
  let voice: ReturnType<typeof createCompanionVoice> | undefined;
  let visible = $state(true);
  let active = $state(false);
  let voiceIntent = 0;
  let captureIntent = 0;
  let captureTaskId = $state<string | null>(null);

  const selectedTask = $derived(eligibleTasks.find(task => task.id === selectedTaskId) ?? null);
  const draft = $derived(selectedTaskId ? draftsByTask[selectedTaskId] ?? '' : '');
  const agentFor = (task: Task | undefined) => task ? agents.find(agent => agent.id === task.agentId) : undefined;
  const taskLabel = (taskId: string) => {
    const task = tasks.find(item => item.id === taskId);
    const agent = agentFor(task);
    return task ? `${task.title || 'Untitled chat'} · ${agent?.name ?? task.provider}` : 'Unknown chat';
  };
  const selectedApprovals = $derived(approvals.filter(request => request.status === 'pending' && request.taskId === selectedTaskId));
  const otherPendingCount = $derived(approvals.filter(request => request.status === 'pending' && request.taskId !== selectedTaskId && eligibleTasks.some(task => task.id === request.taskId)).length);
  const replyInbox = createCompanionReplyInbox();
  let replyPopup = $state<{ message: Message | null; remaining: number }>({ message: null, remaining: 0 });
  const capturing = $derived(['arming', 'waiting', 'listening', 'finishing'].includes(voiceState.phase));

  function liveTranscript() {
    return captureIntent === voiceIntent && ['arming', 'waiting', 'listening', 'finishing'].includes(voiceState.phase)
      ? voiceState.transcript.trim() : '';
  }
  function stopVoice(preserve = true) {
    const live = liveTranscript();
    if (preserve && captureTaskId && live) {
      const existing = draftsByTask[captureTaskId]?.trim() ?? '';
      setDraft(captureTaskId, [existing, live].filter(Boolean).join('\n'));
    }
    voiceIntent += 1;
    voiceEnabled = false;
    voice?.stop();
    captureTaskId = null;
  }
  function closeCompanion() { stopVoice(); active = false; setCompanionActive(false); onclose(); }
  function setDraft(taskId: string | null, value: string) {
    if (!taskId) return;
    if (value.trim()) deliveryNotice = '';
    draftsByTask[taskId] = value;
    saveCompanionDraft(taskId, value);
  }

  async function sendCurrent(text = draft, taskId = selectedTaskId) {
    if (!taskId || !connected || sending || !text.trim()) return false;
    sending = true;
    sendError = '';
    deliveryNotice = '';
    try {
      await onsend(taskId, text.trim());
      deliveryNotice = `Sent to ${taskLabel(taskId)}.`;
      editingReview = false;
      if (taskId && (draftsByTask[taskId] ?? '').trim() === text.trim()) setDraft(taskId, '');
      return true;
    } catch (error) {
      sendError = error instanceof Error ? error.message : String(error);
      return false;
    } finally {
      sending = false;
    }
  }
  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const taskId = selectedTaskId;
    const text = displayedOutgoingText();
    if (taskId && text.trim()) setDraft(taskId, text);
    stopVoice(false);
    await sendCurrent(text, taskId);
  }
  async function resolveApproval(request: ApprovalRequest, decision: ApprovalDecision) {
    if (resolvingId || !connected) return;
    resolvingId = request.id; approvalError = '';
    try { await onresolve(request, decision); }
    catch (error) { approvalError = error instanceof Error ? error.message : String(error); }
    finally { resolvingId = null; }
  }
  async function submitApprovalInput(request: ApprovalRequest, response: unknown) {
    if (resolvingId || !connected) return;
    resolvingId = request.id; approvalError = '';
    try { await oninput(request, response); }
    catch (error) { approvalError = error instanceof Error ? error.message : String(error); }
    finally { resolvingId = null; }
  }
  async function handleTranscript(text: string, taskId: string, source: 'wake' | 'manual', autoSendReady = false) {
    if (captureIntent !== voiceIntent || !active || !visible || !connected || taskId !== selectedTaskId || !text.trim()) return;
    const existing = draftsByTask[taskId]?.trim() ?? '';
    const outgoing = existing ? `${existing}\n${text.trim()}` : text.trim();
    setDraft(taskId, outgoing);
    const shouldAutoSend = autoSend && autoSendReady && !existing;
    const intent = voiceIntent;
    if (!shouldAutoSend) { stopVoice(); return; }
    const accepted = await sendCurrent(outgoing, taskId);
    if (source === 'wake' && accepted && intent === voiceIntent && voiceEnabled && voice && selectedTaskId === taskId && connected && visible && active) {
      try { await voice.listen(taskId, autoSend); }
      catch (error) { if (intent === voiceIntent) voiceState = { ...voiceState, phase: 'error', error: error instanceof Error ? error.message : String(error) }; }
    }
    if (!accepted && intent === voiceIntent) stopVoice();
  }
  function handleAutoSendChange(event: Event) {
    const next = (event.currentTarget as HTMLInputElement).checked;
    stopVoice();
    autoSend = next;
  }
  function displayedOutgoingText() {
    const typed = draft.trim();
    const live = captureTaskId === selectedTaskId ? liveTranscript() : '';
    return [typed, live].filter(Boolean).join('\n');
  }
  function editOutgoing(event: Event) {
    const text = (event.currentTarget as HTMLTextAreaElement).value;
    stopVoice(false);
    setDraft(selectedTaskId, text);
    sendError = '';
  }

  function toggleFaceVoice() {
    if (voiceState.phase === 'listening') { void finishVoice(); return; }
    if (voiceState.phase === 'waiting' || voiceState.phase === 'arming') { stopVoice(); return; }
    if (voiceState.phase === 'finishing') return;
    editingReview = false;
    void tapToTalk();
  }
  function beginReviewEdit() {
    const text = displayedOutgoingText();
    stopVoice(false);
    if (selectedTaskId) setDraft(selectedTaskId, text);
    editingReview = true;
  }

  async function enableWake() {
    if (!voice || !voiceAvailable || !selectedTaskId || !connected || !visible) return;
    stopVoice();
    deliveryNotice = '';
    const intent = ++voiceIntent;
    const target = selectedTaskId;
    captureIntent = intent;
    captureTaskId = target;
    voiceEnabled = true;
    voiceState = { phase: 'arming', transcript: '', error: '' };
    const accepted = setCompanionActive(true);
    if (!accepted) {
      voiceEnabled = false;
      voiceState = { phase: 'error', transcript: '', error: 'The iOS voice listener could not be enabled.' };
      return;
    }
    active = true;
    try { await voice.listen(target, autoSend); }
    catch (error) { if (intent !== voiceIntent) return; voiceEnabled = false; voiceState = { phase: 'error', transcript: '', error: error instanceof Error ? error.message : String(error) }; }
  }
  async function tapToTalk() {
    if (!voice || !voiceAvailable || !selectedTaskId || !connected || !visible) return;
    stopVoice();
    deliveryNotice = '';
    const intent = ++voiceIntent;
    const target = selectedTaskId;
    captureIntent = intent;
    captureTaskId = target;
    voiceEnabled = false;
    setCompanionActive(true);
    active = true;
    voiceState = { phase: 'arming', transcript: '', error: '' };
    try { await voice.record(target, autoSend); }
    catch (error) { if (intent !== voiceIntent) return; voiceState = { phase: 'error', transcript: '', error: error instanceof Error ? error.message : String(error) }; }
  }
  async function finishVoice() {
    try { await voice?.finish(); }
    catch (error) { voiceState = { ...voiceState, phase: 'error', error: error instanceof Error ? error.message : String(error) }; }
  }

  $effect(() => {
    const nextTaskId = selectedTaskId;
    if (nextTaskId !== previousTaskId) {
      previousTaskId = nextTaskId;
      editingReview = false;
      untrack(() => stopVoice());
    }
  });
  $effect(() => {
    if (!connected) untrack(() => stopVoice());
  });
  $effect(() => {
    eligibleTasks;
    if (!selectedTaskId || !eligibleTasks.some(task => task.id === selectedTaskId)) selectedTaskId = recentTaskId();
  });
  $effect(() => {
    const currentMessages = messages;
    const currentTasks = eligibleTasks;
    const pinnedTaskId = selectedTaskId;
    untrack(() => { replyPopup = replyInbox.sync(currentMessages, currentTasks, pinnedTaskId); });
  });

  function dismissReply() { replyPopup = replyInbox.dismiss(); }

  onMount(() => {
    voiceAvailable = isCompanionAvailable();
    voice = createCompanionVoice({
      onlevel: level => { voiceLevel = level; },
      onstate: state => {
        const cancelled = state.phase === 'off' && !state.transcript && voiceState.phase === 'listening';
        voiceState = state;
        if (state.phase === 'error') {
          voiceEnabled = false;
          if (captureIntent === voiceIntent && captureTaskId && state.transcript.trim()) {
            const existing = draftsByTask[captureTaskId]?.trim() ?? '';
            setDraft(captureTaskId, existing ? `${existing}\n${state.transcript.trim()}` : state.transcript.trim());
          }
          captureTaskId = null;
        } else if (cancelled) voiceEnabled = false;
      },
      ontranscript: (text, taskId, source, autoSendReady = false) => { void handleTranscript(text, taskId, source, autoSendReady); }
    });
    active = true;
    setCompanionActive(true);
    const updateVisibility = () => {
      visible = document.visibilityState !== 'hidden';
      if (!visible) { stopVoice(); active = false; setCompanionActive(false); }
      else { active = true; setCompanionActive(true); }
    };
    document.addEventListener('visibilitychange', updateVisibility);
    return () => {
      voiceEnabled = false;
      voiceIntent += 1;
      document.removeEventListener('visibilitychange', updateVisibility);
      voice?.destroy();
      active = false;
      setCompanionActive(false);
    };
  });
</script>

<svelte:head><title>Mo · Monitter</title><meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover" /></svelte:head>

<main class="companion" class:reviewing={!!displayedOutgoingText().trim()} use:mobileViewport aria-label="Monitter desk companion">
  <div class="corner-controls">
    <div class="connection" class:offline={!connected} role="status" aria-label={connected ? 'Desktop connected' : 'Desktop offline'}><i></i></div>
    <button class="close" type="button" aria-label="Close Companion Mode" onclick={closeCompanion}><X size={18}/></button>
  </div>

  <div class="workspace-shell">
  <div class="workspace">
    <section class="hero" aria-label="Mo, your Monitter companion and chat selection">
      <button class="avatar" class:active={selectedTask?.status === 'running' || capturing} class:listening={capturing} class:speaking={capturing && voiceLevel > .12} class:error={selectedTask?.status === 'error' || voiceState.phase === 'error'} class:attention={selectedApprovals.length > 0} style={`--voice-level: ${voiceLevel}`} type="button" onclick={toggleFaceVoice} aria-pressed={capturing} aria-label={voiceState.phase === 'listening' ? 'Stop listening' : voiceState.phase === 'waiting' ? 'Stop listening for Hey Mo' : voiceState.phase === 'arming' ? 'Cancel listening' : voiceState.phase === 'finishing' ? 'Finishing transcription' : 'Tap to talk'} disabled={!selectedTaskId || !connected || !voiceAvailable || sending || voiceState.phase === 'finishing'}>
        <span class="avatar-core" aria-hidden="true"><span class="face"><i></i><i></i></span><span class="mouth"></span></span>
      </button>
      <div class="face-status" aria-live="polite">{!connected ? 'Desktop offline' : !voiceAvailable ? 'Voice is unavailable here' : voiceState.phase === 'arming' ? 'Starting microphone…' : voiceState.phase === 'waiting' ? 'Listening for Hey Mo' : voiceState.phase === 'listening' ? 'Listening · tap Mo to finish' : voiceState.phase === 'finishing' ? 'Finishing…' : voiceState.phase === 'error' ? voiceState.error : selectedApprovals.length ? 'Waiting for your approval' : selectedTask?.status === 'running' ? 'Working now' : selectedTask?.status === 'error' ? 'Run needs attention' : 'Tap Mo to talk'}</div>
      <label class="wake-toggle"><input type="checkbox" role="switch" aria-label="Hey Mo" checked={voiceEnabled} disabled={!voiceAvailable || !connected || !selectedTaskId || sending || (!voiceEnabled && ['listening','finishing'].includes(voiceState.phase))} onchange={event => event.currentTarget.checked ? void enableWake() : stopVoice()} /><span class="switch-track" aria-hidden="true"></span><span>Hey Mo</span></label>
      <label class="auto-send-toggle"><input type="checkbox" checked={autoSend} onchange={handleAutoSendChange}/><span class="switch-track" aria-hidden="true"></span><span>Auto-send after a 10-second pause</span></label>
      <label class="chat-picker"><span>PINNED CHAT</span>
        <select aria-label="Pinned chat" value={selectedTaskId ?? ''} onchange={event => selectedTaskId = event.currentTarget.value || null}>
          {#if !eligibleTasks.length}<option value="">No active chats</option>{/if}
          {#each [...eligibleTasks].sort((a, b) => b.updatedAt - a.updatedAt) as task (task.id)}
            <option value={task.id}>{task.title || 'Untitled chat'} — {agentFor(task)?.name ?? task.provider}</option>
          {/each}
        </select>
      </label>
    </section>

    {#if selectedApprovals.length || otherPendingCount > 0 || approvalError}<section class="inbox" aria-label="Requests needing attention">
      {#if selectedApprovals.length || otherPendingCount > 0}<div class="section-heading"><div><p class="eyebrow">AT YOUR DESK</p><h2>Needs your attention</h2></div>
        {#if otherPendingCount > 0}<button class="other-count" type="button" onclick={() => { const next = approvals.find(item => item.status === 'pending' && item.taskId !== selectedTaskId && eligibleTasks.some(task => task.id === item.taskId)); if (next) selectedTaskId = next.taskId; }}>{otherPendingCount} in other chats <span>→</span></button>{/if}
      </div>{/if}
      {#if selectedApprovals.length}
        {#each selectedApprovals as request (request.id)}
          <article class="request-card"><div class="task-label"><span>CHAT</span>{taskLabel(request.taskId)}</div>
            <ApprovalDock requests={[request]} disabled={!connected || resolvingId !== null} resolvingId={resolvingId} onresolve={resolveApproval} oninput={submitApprovalInput}/>
          </article>
        {/each}
      {/if}
      {#if approvalError}<p class="send-error" role="alert">{approvalError}</p>{/if}

    </section>{/if}
  </div>
  {#if replyPopup.message}
    <aside class="reply-popup" aria-label="Agent reply">
      <header class="reply-popup-header">
        <div class="reply-popup-source"><span>AGENT REPLY</span><strong>{taskLabel(replyPopup.message.taskId)}</strong></div>
        {#if replyPopup.remaining > 0}<span class="reply-popup-count">+{replyPopup.remaining} more</span>{/if}
        <button type="button" aria-label="Dismiss agent reply" onclick={dismissReply}><X size={17}/></button>
      </header>
      <!-- svelte-ignore a11y_no_noninteractive_tabindex (long replies need a keyboard-focusable scroll region) -->
      <div class="reply-popup-content" role="region" aria-label="Reply content" tabindex="0">
        <Markdown text={replyPopup.message.text} taskId={replyPopup.message.taskId}/>
      </div>
      {#if replyPopup.message.taskId !== selectedTaskId}
        <button class="reply-in-chat" type="button" onclick={() => selectedTaskId = replyPopup.message!.taskId}>Reply in this chat</button>
      {/if}
    </aside>
  {/if}
  </div>

  {#if displayedOutgoingText().trim() || sendError || deliveryNotice}
    <footer class="composer-wrap">
      {#if sendError}<p class="send-error" role="alert">{sendError}</p>{/if}
      {#if displayedOutgoingText().trim()}
        <form class="composer" onsubmit={submit}>
          <div class="outgoing-bubble">
            <div class="outgoing-label">YOUR MESSAGE · {taskLabel(selectedTaskId ?? '')}</div>
            {#if editingReview}<textarea aria-label="Edit message" value={displayedOutgoingText()} oninput={editOutgoing} rows="3" disabled={sending}></textarea>
            {:else}<p class="outgoing-text">{displayedOutgoingText()}</p>{/if}
            <div class="review-actions">
              {#if !editingReview}<button class="edit-review" type="button" onclick={beginReviewEdit} disabled={sending}>Edit</button>{/if}
              <button class="send-large" type="submit" aria-label="Send message" disabled={!selectedTaskId || !connected || sending}><Send size={24}/><span>{sending ? 'Sending…' : 'Send'}</span></button>
            </div>
          </div>
        </form>
      {:else if deliveryNotice}<p class="delivery-notice" role="status">{deliveryNotice}</p>{/if}
    </footer>
  {/if}
</main>

<style>
  :global(html:has(.companion)), :global(body:has(.companion)) { margin:0; min-width:320px; overflow:hidden!important; background:var(--paper,#101214); }
  .companion { --c-bg:var(--paper,#101214); --c-panel:var(--panel,#191c1e); --c-line:var(--line,#303638); --c-ink:var(--ink,#eee); --c-muted:var(--muted,#9da5a4); --c-accent:var(--accent,#72d7bd); position:fixed; inset:0; top:var(--mobile-viewport-top,0px); z-index:1200; display:flex; flex-direction:column; box-sizing:border-box; height:var(--mobile-viewport-height,100dvh); min-height:0; overflow:hidden; padding:env(safe-area-inset-top) max(16px,env(safe-area-inset-right)) env(safe-area-inset-bottom) max(16px,env(safe-area-inset-left)); background:var(--c-bg); color:var(--c-ink); font-family:var(--interface-font,"IBM Plex Sans",-apple-system,BlinkMacSystemFont,sans-serif); }
  button,input,select { font:inherit; }
  button { cursor:pointer; }
  button:disabled { cursor:default; opacity:.5; }
  button:focus-visible,select:focus-visible,.reply-popup-content:focus-visible { outline:2px solid var(--c-accent); outline-offset:3px; }
  .corner-controls { position:absolute; z-index:20; inset:env(safe-area-inset-top) max(8px,env(safe-area-inset-right)) auto max(8px,env(safe-area-inset-left)); height:44px; pointer-events:none; }
  .connection { position:absolute; top:18px; left:12px; }
  .connection i { display:block; width:6px; height:6px; border-radius:50%; background:var(--c-accent); opacity:.65; }
  .connection.offline i { background:#d88675; }
  .close { position:absolute; top:0; right:0; display:grid; place-items:center; width:44px; height:44px; border:0; border-radius:50%; background:transparent; color:var(--c-muted); opacity:.6; pointer-events:auto; }
  .close:focus-visible,.close:hover { opacity:1; }
  .workspace-shell { position:relative; flex:1; min-height:0; overflow:hidden; }
  .workspace { box-sizing:border-box; width:min(760px,100%); height:100%; margin:0 auto; padding:44px 0 12px; overflow-y:auto; overscroll-behavior:contain; }
  .hero { display:flex; flex-direction:column; align-items:center; justify-content:center; box-sizing:border-box; min-height:100%; gap:8px; text-align:center; }
  .avatar { appearance:none; -webkit-appearance:none; position:relative; display:grid; place-items:center; flex:none; box-sizing:border-box; width:calc(100% - 12px); max-width:600px; height:calc(var(--mobile-viewport-height,100dvh) * .52); margin:0 0 4px; padding:0; border:0; border-radius:35% 38% 36% 39%; background:transparent; }
  .avatar-core { position:relative; display:block; width:60%; height:60%; border-radius:inherit; background:linear-gradient(145deg,color-mix(in srgb,var(--c-accent) 66%,white),var(--c-accent)); box-shadow:0 8px 44px color-mix(in srgb,var(--c-accent) 18%,transparent),inset 0 1px 1px #fff4; transition:transform 90ms ease-out; }
  .face { position:absolute; top:36%; left:50%; display:flex; gap:clamp(25px,7.8vw,60px); transform:translateX(-50%); }
  .face i { display:block; width:clamp(8px,2.82vw,18px); height:clamp(14px,4.2vw,26px); border-radius:50%; background:#163a33; transform-origin:center; animation:blink 5.5s ease-in-out infinite; }
  .face i:nth-child(2) { animation-delay:.08s; }
  .mouth { position:absolute; top:65%; left:50%; width:clamp(20px,6vw,41px); height:clamp(7px,2.1vw,14px); transform:translateX(-50%); border-bottom:2px solid #235344; border-radius:0 0 50% 50%; transition:height 90ms ease-out,background 90ms ease-out,border-radius 90ms ease-out; }
  .avatar:not(.active) .avatar-core { animation:idle-float 4s ease-in-out infinite; }
  .avatar.active .avatar-core { animation:breathe 2s ease-in-out infinite; }
  .avatar.speaking .avatar-core { animation:none; transform:scale(calc(1 + var(--voice-level) * .045)); }
  .avatar.speaking .mouth { height:calc(clamp(7px,2.1vw,14px) + var(--voice-level) * 28px); border:0; border-radius:50%; background:#235344; }
  .avatar.listening .avatar-core { box-shadow:0 0 0 3px var(--c-bg),0 0 0 6px var(--c-accent),0 8px 44px color-mix(in srgb,var(--c-accent) 30%,transparent); }
  .avatar.error .avatar-core { background:linear-gradient(145deg,#d8a29a,#b66d64); }
  .avatar.attention:not(.listening) .avatar-core { box-shadow:0 0 0 3px #d3a55f; }
  .avatar:active .avatar-core { transform:scale(.985); }
  .face-status { min-height:17px; max-width:95%; color:var(--c-muted); font-size:12px; line-height:1.4; overflow-wrap:anywhere; }
  .wake-toggle,.auto-send-toggle { position:relative; display:flex; flex:none; align-items:center; justify-content:center; gap:12px; box-sizing:border-box; width:100%; min-height:50px; padding:10px 14px; border:1px solid var(--c-line); border-radius:16px; background:color-mix(in srgb,var(--c-panel) 65%,transparent); color:var(--c-ink); cursor:pointer; }
  .wake-toggle { font-size:20px; font-weight:550; }
  .auto-send-toggle { font-size:14px; }
  .wake-toggle input,.auto-send-toggle input { position:absolute; width:1px; height:1px; margin:0; opacity:0; }
  .switch-track { position:relative; display:block; flex:none; width:38px; height:23px; border:1px solid var(--c-line); border-radius:20px; background:color-mix(in srgb,var(--c-muted) 24%,var(--c-panel)); transition:background .15s; }
  .switch-track::after { content:''; position:absolute; top:3px; left:3px; width:17px; height:17px; border-radius:50%; background:var(--c-ink); transition:transform .15s; }
  input:checked + .switch-track { border-color:var(--c-accent); background:var(--c-accent); }
  input:checked + .switch-track::after { transform:translateX(15px); background:var(--on-accent,#fff); }
  input:focus-visible + .switch-track { outline:2px solid var(--c-accent); outline-offset:3px; }
  input:disabled ~ span { opacity:.4; }
  .chat-picker { display:flex; align-items:center; gap:7px; max-width:94%; min-height:28px; color:var(--c-muted); }
  .chat-picker>span { flex:none; font:8px var(--mono,monospace); letter-spacing:.1em; }
  .chat-picker select { min-width:0; max-width:min(65vw,450px); padding:4px 0; border:0; background:transparent; color:var(--c-muted); font-size:11px; text-overflow:ellipsis; }
  .chat-picker option { background:var(--c-panel); color:var(--c-ink); }
  .inbox { margin-top:18px; padding-bottom:8px; }
  .section-heading { display:flex; align-items:end; justify-content:space-between; gap:10px; margin-bottom:10px; }
  .eyebrow { margin:0; color:var(--c-muted); font:8px var(--mono,monospace); letter-spacing:.12em; }
  h2 { margin:4px 0 0; font-size:16px; font-weight:550; }
  .other-count { padding:8px; border:1px solid var(--c-line); border-radius:9px; background:var(--c-panel); color:var(--c-ink); font-size:11px; }
  .request-card { margin-bottom:9px; padding:10px; border:1px solid var(--c-line); border-radius:12px; background:var(--c-panel); }
  .task-label { display:flex; align-items:center; gap:7px; margin-bottom:7px; color:var(--c-muted); font-size:11px; overflow-wrap:anywhere; }
  .task-label>span { flex:none; font:8px var(--mono,monospace); }
  .request-card :global(.approval-dock) { width:100%; max-width:none; max-height:360px; margin:0; box-shadow:none; }
  .reply-popup { position:absolute; z-index:8; top:38px; left:50%; display:flex; flex-direction:column; box-sizing:border-box; width:min(660px,calc(100% - 16px)); max-height:min(32dvh,calc(100% - 90px)); transform:translateX(-50%); overflow:hidden; border:1px solid color-mix(in srgb,var(--c-accent) 35%,var(--c-line)); border-radius:22px 22px 22px 6px; background:var(--c-panel); color:var(--c-ink); box-shadow:0 15px 45px #0005; }
  .reply-popup-header { display:flex; flex:none; align-items:center; gap:8px; padding:7px 10px; border-bottom:1px solid var(--c-line); }
  .reply-popup-source { display:grid; gap:3px; min-width:0; }
  .reply-popup-source>span { color:var(--c-accent); font:8px var(--mono,monospace); letter-spacing:.1em; }
  .reply-popup-source>strong { overflow:hidden; color:var(--c-ink); font-size:11px; font-weight:550; text-overflow:ellipsis; white-space:nowrap; }
  .reply-popup-count { flex:none; color:var(--c-muted); font-size:10px; }
  .reply-popup-header button { display:grid; place-items:center; flex:none; width:44px; height:44px; margin-left:auto; padding:0; border:0; border-radius:50%; background:transparent; color:var(--c-muted); }
  .reply-popup-content { flex:1; min-height:0; padding:12px 14px; overflow-y:auto; overscroll-behavior:contain; font-size:16px; line-height:1.5; }
  .reply-popup-content :global(.markdown) { overflow:visible; }
  .reply-popup-content :global(pre) { max-width:100%; overflow:auto!important; white-space:pre; }
  .reply-in-chat { flex:none; min-height:44px; margin:0 12px 12px; padding:8px 12px; border:1px solid var(--c-line); border-radius:10px; background:color-mix(in srgb,var(--c-accent) 12%,var(--c-panel)); color:var(--c-ink); font-size:13px; }
  .composer-wrap { position:relative; z-index:15; flex:none; box-sizing:border-box; width:min(760px,100%); margin:0 auto; padding:8px 0; }
  .composer { margin:0; }
  .outgoing-bubble { padding:12px 14px; border:1px solid var(--c-line); border-radius:20px 20px 6px 20px; background:var(--c-panel); box-shadow:0 8px 28px #0002; }
  .outgoing-label { margin-bottom:7px; color:var(--c-muted); font:9px var(--mono,monospace); letter-spacing:.08em; overflow:hidden; white-space:nowrap; text-overflow:ellipsis; }
  .outgoing-text { max-height:110px; margin:0; overflow:auto; white-space:pre-wrap; overflow-wrap:anywhere; font-size:18px; line-height:1.45; }
  .outgoing-bubble textarea { display:block; box-sizing:border-box; width:100%; max-height:110px; resize:none; border:0; background:transparent; color:var(--c-ink); font:16px/1.5 var(--interface-font,sans-serif); }
  .review-actions { display:flex; align-items:center; gap:10px; margin-top:10px; }
  .edit-review { flex:none; min-width:62px; min-height:44px; border:1px solid var(--c-line); border-radius:11px; background:transparent; color:var(--c-ink); font-size:14px; }
  .send-large { display:flex; flex:1; align-items:center; justify-content:center; gap:10px; min-height:76px; padding:12px; border:0; border-radius:16px; background:var(--c-accent); color:var(--on-accent,#12231c); font-size:26px; font-weight:600; }
  .delivery-notice { margin:0; padding:5px 10px; color:var(--c-muted); font-size:11px; text-align:center; }
  .send-error { margin:0 0 6px; color:#c75b4c; font-size:12px; overflow-wrap:anywhere; }
  .reviewing .hero { min-height:0; }
  .companion:global([data-keyboard-composer="true"]) .hero { display:none; }
  .companion:global([data-keyboard-composer="true"]) .workspace { padding-top:30px; }
  .companion:global([data-keyboard-composer="true"]) .reply-popup { top:28px; max-height:calc(100% - 30px); }
  @media(max-height:740px) and (orientation:portrait) { .hero { gap:5px; } .wake-toggle,.auto-send-toggle { min-height:44px; padding-block:8px; } .workspace { padding-top:40px; } }
  @media(orientation:landscape) and (max-height:560px) {
    .hero { display:grid; grid-template-columns:minmax(0,1fr) minmax(0,1fr); grid-template-rows:repeat(4,auto); align-content:center; justify-items:center; column-gap:24px; }
    .avatar { grid-column:1; grid-row:1/4; width:95%; height:58dvh; }
    .face-status { grid-column:1; grid-row:4; }
    .wake-toggle { grid-column:2; grid-row:1; }
    .auto-send-toggle { grid-column:2; grid-row:2; }
    .chat-picker { grid-column:2; grid-row:3; width:100%; }
    .chat-picker select { max-width:28vw; }
    .face { gap:39px; } .face i { width:11px; height:17px; } .mouth { width:24px; height:8px; }
    .reply-popup { top:34px; max-height:calc(100% - 45px); width:min(560px,70%); }
    .reviewing .avatar { min-height:0; height:20dvh; }
    .reviewing .hero { gap:5px 20px; }
    .outgoing-text,.outgoing-bubble textarea { max-height:62px; }
    .send-large { min-height:56px; font-size:22px; }
  }
  @keyframes breathe { 0%,100% { transform:scale(1); } 50% { transform:scale(1.01); } }
  @keyframes idle-float { 0%,100% { transform:translateY(0); } 50% { transform:translateY(-4px); } }
  @keyframes blink { 0%,92%,96%,100% { transform:scaleY(1); } 94% { transform:scaleY(.08); } }
  @media(prefers-reduced-motion:reduce) { *,*::before,*::after { animation:none!important; transition:none!important; scroll-behavior:auto!important; } .avatar.speaking .avatar-core { transform:none; } .avatar.speaking .mouth { height:16px; } }
</style>
