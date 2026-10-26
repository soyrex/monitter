<script lang="ts">
  import { onMount, tick } from 'svelte';
  import Menu from "@lucide/svelte/icons/menu";
  import ArrowUp from "@lucide/svelte/icons/arrow-up";
  import Archive from "@lucide/svelte/icons/archive";
  import Bot from "@lucide/svelte/icons/bot";
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import ChevronRight from "@lucide/svelte/icons/chevron-right";
  import CircleStop from "@lucide/svelte/icons/circle-stop";
  import Folder from "@lucide/svelte/icons/folder";
  import LayoutList from "@lucide/svelte/icons/layout-list";
  import ListFilter from "@lucide/svelte/icons/list-filter";
  import MessageCircle from "@lucide/svelte/icons/message-circle";
  import Monitor from "@lucide/svelte/icons/monitor";
  import QrCode from "@lucide/svelte/icons/qr-code";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import Square from "@lucide/svelte/icons/square";
  import Unplug from "@lucide/svelte/icons/unplug";
  import type { Message, SidebarView, Snapshot, Task, TerminalSession } from '$lib/types';
  import { createResumableMobileSession, type ResumableMobileSession } from '$lib/controller/resumable-session';
  import { watchReconnectEvents } from '$lib/controller/reconnect-events';
  import { clearMobilePairing, getOrCreateMobileIdentity, loadMobilePairing, saveMobilePairing, type MobilePairingRecord } from '$lib/controller/pairing-store';
  import { redeemPairingCode } from '$lib/controller/pairing-code';
  import MessageMeta from '$lib/components/MessageMeta.svelte';
  import MobilePreferences from '$lib/components/MobilePreferences.svelte';
  import ResponseMetadata from '$lib/components/ResponseMetadata.svelte';
  import SparkleField from '$lib/components/SparkleField.svelte';
  import NativeSpeechRecorder from '$lib/components/NativeSpeechRecorder.svelte';
  import { nativeSpeechPlatform } from '$lib/mobile-speech';
  import { contrastForeground } from '$lib/accent-contrast';
  import { loadSidebarViewPreference, saveSidebarViewPreference } from '$lib/sidebar-view-preference';
  import { isCancellationMessage, isContextClearedMessage, modelSettingsChangeLabel } from '$lib/activity-grouping';
  import { interfaceScaleStore, seedViewerInterfaceScale, watchViewerInterfaceScales } from '$lib/interface-scale';
  import ProviderIcon from '$lib/components/ProviderIcon.svelte';
  import { autoGrowTextarea } from '$lib/textarea-autogrow';
  import { projectBoardChannels, projectBoardId } from '$lib/project-board-channels';
  import { isMobileBundleOrigin } from '$lib/lan';
  // Bundled phone shells use this route as the owner-pairing bootstrap. The
  // earlier limited controller remains available through ?controller=1.
  const ownerWorkspaceMode = typeof window !== 'undefined' &&
    !new URLSearchParams(window.location.search).has('controller') &&
    (isMobileBundleOrigin() || new URLSearchParams(window.location.search).get('owner') === '1');

  let deviceKey=$state(''), verificationCode=$state(''), connecting=$state(false), connectionGeneration=0;
  let status=$state('disconnected'), error=$state(''), connectionError=$state(''), remembered=$state(false), snapshot=$state<Snapshot|null>(null);
  let everConnected=$state(false);
  let selected=$state<string|null>(null), selectedChannelId=$state<string|null>(null), selectedTerminalId=$state<string|null>(null), text=$state('');
  // The transport object has identity-sensitive async callbacks. Deep $state
  // would proxy it, making a freshly connected session fail `current === session`.
  let session=$state.raw<ResumableMobileSession|null>(null), unsubscribe:(()=>void)|undefined;
  let pairingEpoch=0, pairingWrites:Promise<void>=Promise.resolve(), preparingPairing=false;
  let ownerRequestPending=$state(false), ownerRequestEpoch=0;
  let ownerUnsupported=$state(false);
  let snapshotEpoch=0, refreshInFlight=false, refreshToken=0, snapshotRequestSeq=0, snapshotAppliedSeq=0, sidebarViewSession=-1, terminalSelectionEpoch=0, operationSeq=0;
  let sendOps=$state<Record<string,number>>({}), stopOps=$state<Record<string,number>>({});
  let chatDrafts=$state<Record<string,string>>({}), menu=$state<'workspace'|'archived'|'terminals'|'preferences'|'hosts'|'agents'>('workspace');
  let androidSpeechBridge=$state(false);
  let mobileView=$state<SidebarView>('standard'), collapsedAgents=$state<Record<string,boolean>>({}), collapsedProjects=$state<Record<string,boolean>>({});
  let terminals=$state<TerminalSession[]>([]), terminalText=$state(''), terminalTruncated=$state(false), terminalSeq=$state(0), terminalReads=$state<Record<string,number>>({}), messageLimit=$state(100), terminalDecoder=new TextDecoder();
  let thread=$state<HTMLElement>(), atBottom=$state(true);
  let olderMessages=$state<Record<string,Message[]>>({}), historyCursors=$state<Record<string,string|null>>({}), historyLoading=$state<Record<string,boolean>>({}), historyErrors=$state<Record<string,string>>({});
  const mobileInterfaceScale=interfaceScaleStore('mobile');

  const tasks=$derived(snapshot?.tasks ?? []);
  const activeTasks=$derived(tasks.filter(task=>!task.archived && !task.channelId));
  const archivedTasks=$derived(tasks.filter(task=>task.archived && !task.channelId));
  const task=$derived(tasks.find(t=>t.id===selected) ?? null);
  const channels=$derived([...(snapshot?.channels ?? []), ...projectBoardChannels(snapshot)]);
  const channel=$derived(channels.find(item=>item.id===selectedChannelId) ?? null);
  const selectedTerminal=$derived(terminals.find(item=>item.id===selectedTerminalId) ?? null);
  const messages=$derived(selected ? [...new Map([...(olderMessages[selected]??[]), ...(snapshot?.messages.filter(m=>m.taskId===selected)??[])].map(message=>[message.id,message])).values()].sort((a,b)=>a.createdAt-b.createdAt) : []);
  const allConversationMessages=$derived(channel ? channel.messages : messages);
  const visibleMessages=$derived(allConversationMessages.slice(-messageLimit));
  const activityTasks=$derived([...activeTasks].sort((a,b)=>Number(b.status==='running')-Number(a.status==='running') || b.updatedAt-a.updatedAt || a.id.localeCompare(b.id)));
  const theme=$derived(snapshot?.settings.theme === 'light' ? 'light' : snapshot?.settings.theme === 'system' && typeof window !== 'undefined' && matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark');
  const workspaceTitle=$derived(menu==='archived' ? 'Archived chats' : menu==='terminals' ? 'Terminals' : menu==='preferences' ? 'Preferences' : menu==='hosts' ? 'Hosts' : menu==='agents' ? 'Agent directory' : 'Your workspace');
  const selectedTitle=$derived(task?.title ?? channel?.name ?? selectedTerminal?.title ?? 'monitter');
  const viewingConversation=$derived(Boolean(task || channel));
  const conversationReadOnly=$derived(Boolean(channel || task?.archived));
  const remoteStyle=$derived.by(()=>{const accent=snapshot?.settings.accent || '#73ce9f';return `--accent:${accent};--on-accent:${contrastForeground(accent)};`;});
  const reconnecting=$derived(everConnected && status!=='connected' && status!=='rejected');

  function selectChat(id:string|null){ if(selected)chatDrafts[selected]=text; messageLimit=100;terminalSelectionEpoch++;selected=id;selectedChannelId=null;selectedTerminalId=null;text=id?chatDrafts[id]??'':''; }
  function appendNativeTranscript(transcript:string, taskId:string){
    if(!taskId || !snapshot?.tasks.some(task=>task.id===taskId && !task.archived))return;
    const previous=selected===taskId?text:(chatDrafts[taskId]??'');
    const next=previous?`${previous.replace(/\s+$/,'')}\n\n${transcript.trim()}`:transcript.trim();
    if(!next)return;
    chatDrafts[taskId]=next;
    if(selected===taskId)text=next;
  }
  function selectChannel(id:string){ if(selected)chatDrafts[selected]=text;messageLimit=100;terminalSelectionEpoch++;selected=null;selectedChannelId=id;selectedTerminalId=null;text=''; }
  async function selectTerminal(id:string){ if(selected)chatDrafts[selected]=text;terminalSelectionEpoch++;terminalDecoder=new TextDecoder();selected=null;selectedChannelId=null;selectedTerminalId=id;text='';terminalText='';terminalTruncated=false;terminalSeq=0;await readTerminal(id,0,true); }
  function backToMenu(){if(selected)chatDrafts[selected]=text;terminalSelectionEpoch++;selected=null;selectedChannelId=null;selectedTerminalId=null;text='';}
  $effect(()=>{selected;selectedChannelId;selectedTerminalId;void tick().then(()=>{if(thread){thread.scrollTop=thread.scrollHeight;atBottom=true;}});});
  $effect(()=>{if(snapshot&&sidebarViewSession!==connectionGeneration){sidebarViewSession=connectionGeneration;mobileView=loadSidebarViewPreference('mobile')??snapshot.settings.sidebarView??'standard';saveSidebarViewPreference('mobile',mobileView);}});
  $effect(()=>{if(snapshot)seedViewerInterfaceScale('mobile',snapshot.settings.interfaceScale??125);});

  function setMobileView(view: SidebarView) {
    if (mobileView === view) return;
    mobileView = view;
    saveSidebarViewPreference('mobile', view);
  }

  const reconnectMessage='Reconnecting to your desktop. Try again once connected.';
  async function loadEarlierMessages(){
    const taskId=selected,current=session;
    if(!taskId||!current||!current.supportsMessagePages()||historyLoading[taskId]||historyCursors[taskId]===null||status!=='connected')return;
    const generation=connectionGeneration, initial=(snapshot?.messages??[]).filter(message=>message.taskId===taskId&&/^[0-9a-f]{8}-[0-9a-f-]{27,}$/i.test(message.id)).sort((a,b)=>a.createdAt-b.createdAt);
    const before=Object.hasOwn(historyCursors,taskId)?historyCursors[taskId]??undefined:initial[0]?.id;
    const oldHeight=thread?.scrollHeight??0,oldTop=thread?.scrollTop??0;
    atBottom=false;historyLoading={...historyLoading,[taskId]:true};historyErrors={...historyErrors,[taskId]:''};
    try{const page=await current.getTaskMessages(taskId,before,40);if(current!==session||generation!==connectionGeneration)return;
      olderMessages={...olderMessages,[taskId]:[...new Map([...(olderMessages[taskId]??[]),...page.messages].map(message=>[message.id,message])).values()]};
      historyCursors={...historyCursors,[taskId]:page.nextBeforeId};messageLimit+=page.messages.length;
      await tick();if(selected===taskId&&thread)thread.scrollTop=oldTop+(thread.scrollHeight-oldHeight);
    }catch(reason){if(current===session&&generation===connectionGeneration)historyErrors={...historyErrors,[taskId]:String(reason)};}
    finally{if(current===session){const next={...historyLoading};delete next[taskId];historyLoading=next;}}
  }
  async function refresh(notify=false){
    if(ownerWorkspaceMode||!session||refreshInFlight)return;
    if(status!=='connected'){if(notify)error=reconnectMessage;return;}
    const current=session, generation=connectionGeneration, epoch=snapshotEpoch, token=++refreshToken, issued=++snapshotRequestSeq;refreshInFlight=true;
    try { const next=await current.getSnapshot(); if(current!==session||generation!==connectionGeneration||epoch!==snapshotEpoch||issued<snapshotAppliedSeq)return; snapshotAppliedSeq=issued;snapshot=next;if(atBottom){await tick();if(thread)thread.scrollTop=thread.scrollHeight;} }
    catch(e){if(notify&&current===session&&generation===connectionGeneration)error=status==='connected'?String(e):reconnectMessage;}
    finally{if(token===refreshToken)refreshInFlight=false;}
  }
  async function refreshTerminals(notify=false){
    if(!session)return;const current=session,generation=connectionGeneration;
    if(status!=='connected'){if(notify)error=reconnectMessage;return;}
    try{const next=await current.listTerminals();if(current===session&&generation===connectionGeneration)terminals=next;}catch(e){if(notify&&current===session&&generation===connectionGeneration)error=status==='connected'?String(e):reconnectMessage;}
  }
  async function readTerminal(id:string,afterSeq:number,notify=false){
    if(!session||terminalReads[id])return;const current=session,generation=connectionGeneration, selectionEpoch=terminalSelectionEpoch, token=++operationSeq;terminalReads={...terminalReads,[id]:token};
    if(status!=='connected'){if(notify)error=reconnectMessage;const next={...terminalReads};delete next[id];terminalReads=next;return;}
    try{const value=await current.readTerminal(id,afterSeq);if(current!==session||generation!==connectionGeneration||selectionEpoch!==terminalSelectionEpoch||selectedTerminalId!==id)return;const appended=value.chunks.map(chunk=>terminalDecoder.decode(new Uint8Array(chunk.data),{stream:true})).join('');const combined=terminalText+appended;terminalTruncated||=combined.length>120000;terminalText=combined.slice(-120000);terminalSeq=value.nextSeq;}
    catch(e){if(notify&&current===session&&generation===connectionGeneration)error=status==='connected'?String(e):reconnectMessage;}
    finally{if(terminalReads[id]===token){const next={...terminalReads};delete next[id];terminalReads=next;}}
  }
  async function openTerminals(){menu='terminals';void refreshTerminals(true);}
  function queuePairingWrite(epoch:number, action:()=>Promise<void>):Promise<boolean> {
    const operation=pairingWrites.catch(()=>{}).then(async()=>{if(epoch!==pairingEpoch)return false;try{await action();return epoch===pairingEpoch;}catch(reason){if(epoch===pairingEpoch)error=`Could not update secure pairing storage: ${String(reason)}`;return false;}});
    pairingWrites=operation.then(()=>{});
    return operation;
  }
  function resetConnectionState(){snapshotEpoch++;refreshToken++;snapshotRequestSeq++;snapshotAppliedSeq=snapshotRequestSeq;refreshInFlight=false;terminalSelectionEpoch++;snapshot=null;selected=null;selectedChannelId=null;selectedTerminalId=null;text='';chatDrafts={};olderMessages={};historyCursors={};historyLoading={};historyErrors={};terminals=[];terminalText='';terminalReads={};sendOps={};stopOps={};ownerUnsupported=false;}
  async function enterOwnerWorkspace(current:ResumableMobileSession, record:MobilePairingRecord, generation:number, persistenceEpoch:number){
    if(ownerRequestPending)return;
    if(!current.supportsOwnerWorkspace()){ownerUnsupported=true;error='Update Monitter on your desktop to use the owner workspace through the relay. Your LAN connection still works.';return;}
    ownerUnsupported=false;error='';
    ownerRequestPending=true;
    const ownerAttempt=++ownerRequestEpoch;
    try{
      await current.requestOwnerWorkspace();
      const active=()=>ownerAttempt===ownerRequestEpoch&&current===session&&generation===connectionGeneration&&persistenceEpoch===pairingEpoch&&current.getStatus()==='connected'&&current.hasOwnerWorkspaceAccess();
      if(!active())return;
      const saved=await queuePairingWrite(persistenceEpoch,async()=>{if(active())await saveMobilePairing({...record,savedAt:Date.now()});});
      if(!saved||!active())return;
      try { localStorage.setItem('monitter-owner-pairing', '1'); } catch { /* URL still selects the approved relay gate. */ }
      window.location.replace('/?monitter-owner-relay=1');
    }catch(reason){if(ownerAttempt===ownerRequestEpoch&&current===session&&generation===connectionGeneration&&persistenceEpoch===pairingEpoch)error=`Full workspace access was not approved: ${String(reason)}`;}
    finally{if(ownerAttempt===ownerRequestEpoch&&current===session&&generation===connectionGeneration&&persistenceEpoch===pairingEpoch)ownerRequestPending=false;}
  }
  async function openPairing(record:MobilePairingRecord, persistAfterApproval:boolean){
    if(connecting)return;const generation=++connectionGeneration, persistenceEpoch=++pairingEpoch;connecting=true;ownerRequestPending=false;error='';connectionError='';remembered=!persistAfterApproval;verificationCode='';resetConnectionState();
    session?.close();unsubscribe?.();session=null;status='connecting';
    try{const next=await createResumableMobileSession({invitation:record.invitation,identity:{keyPair:record.identity.keyPair}});if(generation!==connectionGeneration){next.close();return;}session=next;unsubscribe=next.subscribe(value=>{if(generation!==connectionGeneration)return;if(value.status!=='connected'){ownerRequestEpoch++;ownerRequestPending=false;}status=value.status;verificationCode=value.verificationCode??'';connectionError=value.error??(value.status==='closed'||value.status==='error'?'Connection interrupted. Reconnecting…':'');if(value.status==='connected'){everConnected=true;if(persistAfterApproval){if(next.supportsRememberedDevices()){void queuePairingWrite(persistenceEpoch,async()=>saveMobilePairing({...record,savedAt:Date.now()})).then(saved=>{if(saved&&generation===connectionGeneration)remembered=true;});}else error='This desktop needs an update before it can remember this phone. The current connection remains available until it closes.';}if(ownerWorkspaceMode)void enterOwnerWorkspace(next,record,generation,persistenceEpoch);else void refresh();}if(value.status==='rejected'){connecting=false;everConnected=false;resetConnectionState();error='Desktop declined this connection. Your saved pairing is preserved. Retry when ready.';}});}
    catch(e){if(generation===connectionGeneration){error=String(e);status='error';}}
    finally{if(generation===connectionGeneration)connecting=false;}
  }
  async function connect(scannedInvitation?:string){
    if(connecting||preparingPairing)return;const current=++connectionGeneration;preparingPairing=true;
    try{const invitation=scannedInvitation??await redeemPairingCode(deviceKey);const record:MobilePairingRecord={version:1,invitation,identity:await getOrCreateMobileIdentity(),savedAt:Date.now()};if(current!==connectionGeneration)return;await openPairing(record,true);}catch(e){if(current===connectionGeneration){error=String(e);status='error';}}finally{preparingPairing=false;}
  }
  async function retryPairing(){error='';if(session){await session.reconnectNow();}else{const saved=await loadMobilePairing();if(saved)await openPairing(saved,false);}}
  function disconnect(){const persistenceEpoch=++pairingEpoch;connectionGeneration++;connecting=false;everConnected=false;unsubscribe?.();session?.close();session=null;resetConnectionState();status='disconnected';connectionError='';remembered=false;deviceKey='';verificationCode='';menu='workspace';queuePairingWrite(persistenceEpoch,clearMobilePairing);}
  async function send(){
    if(!session||!selected||!text.trim()||sendOps[selected]||task?.archived)return;
    if(status!=='connected'){error=reconnectMessage;return;}
    const body=text,taskId=selected,current=session,generation=connectionGeneration, token=++operationSeq, issued=++snapshotRequestSeq;error='';sendOps={...sendOps,[taskId]:token};snapshotEpoch++;
    try{const next=await current.sendMessage(taskId,body);if(current!==session||generation!==connectionGeneration)return;snapshotEpoch++;if(isSnapshot(next)&&issued>=snapshotAppliedSeq){snapshotAppliedSeq=issued;snapshot=next;}if(selected===taskId&&text===body)text='';else if(chatDrafts[taskId]===body)chatDrafts[taskId]='';if(!isSnapshot(next))void refresh();}
    catch(e){if(current===session&&generation===connectionGeneration)error=status==='connected'?String(e):reconnectMessage;}
    finally{if(sendOps[taskId]===token){const next={...sendOps};delete next[taskId];sendOps=next;}}
  }
  async function stop(){if(!session||!selected||stopOps[selected])return;if(status!=='connected'){error=reconnectMessage;return;}const taskId=selected,current=session,generation=connectionGeneration,token=++operationSeq,issued=++snapshotRequestSeq;error='';stopOps={...stopOps,[taskId]:token};snapshotEpoch++;try{const next=await current.cancelTask(taskId);if(current===session&&generation===connectionGeneration&&issued>=snapshotAppliedSeq){snapshotEpoch++;snapshotAppliedSeq=issued;snapshot=next;}}catch(e){if(current===session&&generation===connectionGeneration)error=status==='connected'?String(e):reconnectMessage;}finally{if(stopOps[taskId]===token){const next={...stopOps};delete next[taskId];stopOps=next;}}}
  function scan(){error='';const android=(window as any).monitterAndroid;const ios=(window as any).webkit?.messageHandlers?.scanQR;if(android)android.postMessage('scanQR');else if(ios)ios.postMessage(null);else error='Enter the nine-digit device key shown in desktop Remote control below.';}
  function agentName(value:Task){return snapshot?.agents.find(agent=>agent.id===value.agentId)?.name??'Unknown agent';}
  function projectTasks(projectId:string|null){return activityTasks.filter(item=>item.projectId===projectId);}
  function isSnapshot(value: unknown): value is Snapshot { return !!value && typeof value === 'object' && Array.isArray((value as Snapshot).tasks) && Array.isArray((value as Snapshot).agents); }
  onMount(()=>{
    androidSpeechBridge=nativeSpeechPlatform()==='android';
    let disposed=false;
    const stopScaleWatch=watchViewerInterfaceScales();
    const restore=async(force=false)=>{if(disposed||connecting||preparingPairing)return;const current=connectionGeneration;if(session){if(status==='rejected')return;if(force||status!=='connected')try{await session.reconnectNow();}catch(reason){if(current===connectionGeneration)error=String(reason);}return;}try{const record=await loadMobilePairing();if(!record||disposed||current!==connectionGeneration)return;await openPairing(record,false);}catch(reason){if(current===connectionGeneration)error=`Could not restore secure pairing: ${String(reason)}`;}};
    const qr=(event:Event)=>{const value=(event as CustomEvent<string>).detail;if(typeof value==='string'&&value.length<=4096)void connect(value);};
    const qrError=(event:Event)=>{error=String((event as CustomEvent<string>).detail||'Scanner unavailable. Enter the device key instead.');};
    (window as any).monitterHandleBack=()=>{if(selected||selectedChannelId||selectedTerminalId){backToMenu();return true;}if(menu!=='workspace'){menu='workspace';return true;}return false;};
    const stopReconnectEvents=watchReconnectEvents(()=>void restore(true));
    window.addEventListener('monitter:qr',qr);window.addEventListener('monitter:qr-error',qrError);
    let pendingPin = '';
    if (ownerWorkspaceMode) try { pendingPin = sessionStorage.getItem('monitter-pairing-pin') ?? ''; sessionStorage.removeItem('monitter-pairing-pin'); } catch { /* Manual PIN entry remains available. */ }
    if (/^\d{9}$/.test(pendingPin)) { deviceKey = pendingPin; void connect(); }
    else void restore();
    const timer=setInterval(()=>{if(!ownerWorkspaceMode&&session&&status==='connected'&&!document.hidden){void refresh();if(selectedTerminalId)void readTerminal(selectedTerminalId,terminalSeq);}},2000);
    return()=>{disposed=true;stopReconnectEvents();stopScaleWatch();clearInterval(timer);window.removeEventListener('monitter:qr',qr);window.removeEventListener('monitter:qr-error',qrError);delete (window as any).monitterHandleBack;connectionGeneration++;session?.close();unsubscribe?.();};
  });
</script>
<svelte:head><title>Monitter Remote</title><meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no, viewport-fit=cover" /></svelte:head>
<div class:light={theme==='light'} class="mobile" style={remoteStyle} style:--interface-font-ratio={(snapshot?.settings.interfaceFontSize ?? 14) / 14} style:--viewer-interface-scale={$mobileInterfaceScale/100} style:--viewer-interface-scale-inverse={100/$mobileInterfaceScale}>
  <div class="mobile-content" aria-busy={reconnecting}>
  <header>{#if selected || selectedChannelId || selectedTerminalId}<button aria-label="Open chats menu" onclick={backToMenu}><Menu size={22}/></button>{/if}<div><strong>{selected || selectedChannelId || selectedTerminalId ? selectedTitle : workspaceTitle}</strong>{#if !reconnecting}<small><span class:online={status==='connected'}></span>{status.replaceAll('_',' ')}</small>{/if}</div>{#if session}<button aria-label="Refresh" onclick={async()=>{void refresh(true);if(menu==='terminals')void refreshTerminals(true);}}><RefreshCw size={20}/></button><button aria-label="Forget desktop" title="Forget desktop" onclick={disconnect}><Unplug size={20}/></button>{/if}</header>
  {#if error&&(!reconnecting||error===reconnectMessage)}<div class="error" role="alert">{error}</div>{/if}{#if connectionError&&!reconnecting}<div class="error" role="status">{connectionError}</div>{/if}
  {#if ownerWorkspaceMode || !snapshot}
    <main class="pair">
      <div class="mark" aria-label="Monitter"><svg viewBox="0 0 48 48" width="42" height="42" fill="none" aria-hidden="true"><path d="M8 35V13l16 17 16-17v22" stroke="currentColor" stroke-width="5" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
      <small class="brand-name">MONITTER</small><h1>Your desktop.<br/>In your pocket.</h1>
      {#if status==='awaiting_approval'}<p>Compare this number with your desktop, then approve this phone.</p><strong class="verification">{verificationCode}</strong>
      {:else if ownerWorkspaceMode && ownerUnsupported}<p>Update Monitter on your desktop to open your full workspace here.</p>
      {:else if ownerWorkspaceMode && status==='connected'}<p>Approve full workspace access on your desktop.</p>
      {:else if remembered}<p class="saved-pairing">Your desktop is remembered until you forget it here or revoke this phone on your desktop.</p><p>{connecting || status==='waiting_for_peer' ? 'Connecting to your desktop…' : 'Your desktop will reconnect when it is available.'}</p><button class="primary" onclick={()=>void retryPairing()}><RefreshCw size={20}/>Reconnect</button>
      {:else if connecting || status==='waiting_for_peer'}<p>Connecting… Open Settings → Pair a phone on your desktop.</p>
      {:else}<p>Scan the code in desktop Settings → Pair a phone. Approve once to keep your workspace close.</p><button class="primary" onclick={scan}><QrCode size={20}/>Scan desktop QR code</button><details class="pin-fallback"><summary>Use a pairing PIN instead</summary><label>Desktop pairing PIN<input aria-label="Pairing PIN" inputmode="numeric" autocomplete="one-time-code" maxlength="11" bind:value={deviceKey} placeholder="123 456 789" /></label><button class="secondary" disabled={!/^\d{9}$/.test(deviceKey.replace(/[\s-]/g,''))||connecting} onclick={()=>connect()}>Connect to desktop</button></details><small>Your phone reconnects securely wherever you are. Work stays on your desktop.</small>{/if}
      {#if session && status!=='connected' && !remembered}<button class="secondary" onclick={()=>void retryPairing()}>Try again</button>{/if}
      {#if remembered || session}<button class="secondary forget" onclick={disconnect}>Forget desktop</button>{/if}
    </main>
  {:else if viewingConversation}
    <main class="thread" bind:this={thread} onscroll={()=>{if(thread)atBottom=thread.scrollHeight-thread.scrollTop-thread.clientHeight<60;}}>{#if channel}<div class="capability-note"><MessageCircle size={16}/><span>Channels are read-only on mobile. Continue this conversation on desktop.</span></div>{/if}{#if task?.archived}<div class="capability-note"><Archive size={16}/><span>This chat is archived. Restore it on desktop to continue.</span></div>{/if}{#if selected&&session?.supportsMessagePages()&&historyCursors[selected]!==null}<button class="load-earlier" disabled={historyLoading[selected]} onclick={loadEarlierMessages}>{historyLoading[selected]?'Loading…':'Load earlier messages'}</button>{#if historyErrors[selected]}<p class="history-error" role="alert">{historyErrors[selected]}</p>{/if}{/if}{#if allConversationMessages.length>visibleMessages.length}<button class="load-earlier" onclick={()=>messageLimit+=100}>Load 100 earlier messages ({allConversationMessages.length-visibleMessages.length} remaining)</button>{/if}{#each visibleMessages as message (message.id)}{#if isContextClearedMessage(message)}<div class="context-cleared-event" role="separator"><span></span><time>{new Date(message.createdAt).toLocaleTimeString([], {hour:'numeric',minute:'2-digit'})} · Context Cleared</time><span></span></div>{:else if modelSettingsChangeLabel(message)}<div class="context-cleared-event" role="separator"><span></span><time>{new Date(message.createdAt).toLocaleTimeString([], {hour:'numeric',minute:'2-digit'})} · {modelSettingsChangeLabel(message)}</time><span></span></div>{:else if isCancellationMessage(message)}<div class="cancellation-event" role="status"><CircleStop size={15} aria-hidden="true"/><MessageMeta name={message.text} createdAt={message.createdAt}/></div>{:else}<article class:user={message.role==='user'} class:final-answer={'phase' in message && message.phase==='final_answer'}><div class="message-bubble"><MessageMeta name={message.role==='user'?'You':channel ? snapshot.agents.find(a=>'agentId' in message && a.id===message.agentId)?.name??'Agent' : agentName(task!)} createdAt={message.createdAt}/><div class="message-body">{message.text}</div></div>{#if message.role==='assistant' && 'responseMetadata' in message && message.responseMetadata}<ResponseMetadata metadata={message.responseMetadata}/>{/if}</article>{/if}{:else}<p class="muted">No messages in this conversation yet.</p>{/each}{#if task?.status==='running'}<p class="working">Agent is working…</p>{/if}</main>
    {#if conversationReadOnly}<div class="read-only">{channel?'Channel messages are view-only on this device.':'Archived chats are view-only on this device.'}</div>{:else}<div class="composer-glow"><SparkleField active={task?.status==='running'} animation={snapshot?.settings.activityAnimation ?? 'sparkles'}/></div><form onsubmit={event=>{event.preventDefault();void send();}}>{#if androidSpeechBridge && selected}{#key selected}<NativeSpeechRecorder contextKey={selected} disabled={!!sendOps[selected]} ontranscript={appendNativeTranscript}/>{/key}{/if}<textarea use:autoGrowTextarea={text} aria-label="Message" placeholder="Message your agent…" bind:value={text} rows="2"></textarea>{#if task?.status==='running'}<button type="button" aria-label="Stop agent" disabled={!!stopOps[task.id]} onclick={stop}><Square size={20}/></button>{/if}<button class="send" aria-label="Send message" disabled={!!sendOps[task?.id??'']||!text.trim()}><ArrowUp size={24}/></button></form>{/if}
  {:else if selectedTerminal}
    <main class="terminal" bind:this={thread}><div class="capability-note"><Monitor size={16}/><span>Terminal output is read-only on mobile.</span></div>{#if terminalTruncated}<p class="view-hint">Showing the most recent 120,000 characters.</p>{/if}<pre>{terminalText || (terminalReads[selectedTerminal.id] ? 'Loading terminal output…' : 'No terminal output yet.')}</pre></main>
  {:else}
    <main class="chats">
      {#if menu==='workspace'}
        <div class="workspace-summary"><div><h1>Your workspace</h1><p class="muted">{activeTasks.filter(t=>t.status==='running').length} running · {snapshot.agents.length} agents</p></div><button class="view-button" aria-label="Change workspace view" onclick={()=>setMobileView(mobileView==='standard'?'activity':mobileView==='activity'?'projects':'standard')}>{#if mobileView==='standard'}<Bot size={18}/>{:else if mobileView==='activity'}<ListFilter size={18}/>{:else}<Folder size={18}/>{/if}<span>{mobileView}</span></button></div>
        <div class="view-tabs" role="tablist" aria-label="Workspace views">{#each ([['standard','Standard',Bot],['activity','Activity',ListFilter],['projects','Projects',Folder]] as const) as [view,label,Icon]}<button role="tab" aria-label={`${label} view`} aria-selected={mobileView===view} class:active={mobileView===view} onclick={()=>setMobileView(view)}><Icon size={16}/>{label}</button>{/each}</div>
        {#if mobileView==='standard'}{#each snapshot.agents as agent}<section><button class="section-head" aria-expanded={!collapsedAgents[agent.id]} onclick={()=>collapsedAgents[agent.id]=!collapsedAgents[agent.id]}><span class="agent-mark" style={`background:${agent.color||'var(--accent)'}`}>{#if agent.avatar}<img src={agent.avatar} alt="" />{:else}<ProviderIcon provider={agent.provider} model={agent.model || agent.acp?.command} size={14}/>{/if}</span><span>{agent.name}<small>{agent.provider}</small></span><span class="count">{activeTasks.filter(t=>t.agentId===agent.id).length}</span>{#if collapsedAgents[agent.id]}<ChevronRight size={18}/>{:else}<ChevronDown size={18}/>{/if}</button>{#if !collapsedAgents[agent.id]}{#each activityTasks.filter(t=>t.agentId===agent.id) as chat}<button class="chat" title={chat.title||'Untitled chat'} onclick={()=>selectChat(chat.id)}><span class="dot" class:running={chat.status==='running'}></span><div><b>{chat.title||'Untitled chat'}</b><small>{chat.status}</small></div><span>›</span></button>{:else}<p class="muted indent">No open chats</p>{/each}{/if}</section>{:else}<p class="muted">No agents on this desktop yet.</p>{/each}
        {:else if mobileView==='activity'}<p class="view-hint">Running first, then most recent.</p>{#each activityTasks as chat}<button class="chat" onclick={()=>selectChat(chat.id)}><span class="dot" class:running={chat.status==='running'}></span><div><b>{chat.title||'Untitled chat'}</b><small>{agentName(chat)} · {chat.status}</small></div><span>›</span></button>{:else}<p class="muted">No chats yet.</p>{/each}
        {:else}{#each snapshot.projects as project}<section><button class="section-head" aria-expanded={!collapsedProjects[project.id]} onclick={()=>collapsedProjects[project.id]=!collapsedProjects[project.id]}><span class="project-mark" style={`background:${project.color||'var(--accent)'}`}><Folder size={14}/></span><span>{project.name}</span><span class="count">{projectTasks(project.id).length}</span>{#if collapsedProjects[project.id]}<ChevronRight size={18}/>{:else}<ChevronDown size={18}/>{/if}</button>{#if !collapsedProjects[project.id]}{#each projectTasks(project.id) as chat}<button class="chat" onclick={()=>selectChat(chat.id)}><span class="dot" class:running={chat.status==='running'}></span><div><b>{chat.title||'Untitled chat'}</b><small>{agentName(chat)} · {chat.status}</small></div><span>›</span></button>{:else}<p class="muted indent">No chats in this project</p>{/each}{/if}</section>{/each}<section><button class="section-head" aria-expanded={!collapsedProjects.unassigned} onclick={()=>collapsedProjects.unassigned=!collapsedProjects.unassigned}><Folder size={16}/><span>No project</span><span class="count">{projectTasks(null).length}</span>{#if collapsedProjects.unassigned}<ChevronRight size={18}/>{:else}<ChevronDown size={18}/>{/if}</button>{#if !collapsedProjects.unassigned}{#each projectTasks(null) as chat}<button class="chat" onclick={()=>selectChat(chat.id)}><span class="dot" class:running={chat.status==='running'}></span><div><b>{chat.title||'Untitled chat'}</b><small>{agentName(chat)} · {chat.status}</small></div><span>›</span></button>{:else}<p class="muted indent">All chats are organised.</p>{/each}{/if}</section>{/if}
        <section class="channels"><h2>Channels</h2>{#each channels as item}<button class="chat" onclick={()=>selectChannel(item.id)}><MessageCircle size={18}/><div><b>{item.name}</b><small>{projectBoardId(item.id) ? 'Private project board · read-only on mobile' : `${item.agentIds.length} agents · read-only on mobile`}</small></div><span>›</span></button>{:else}<p class="muted">No channels yet.</p>{/each}</section>
      {:else if menu==='archived'}<p class="view-hint">Archived chats stay available to read. Restore them on desktop before sending.</p>{#each archivedTasks as chat}<button class="chat" onclick={()=>selectChat(chat.id)}><Archive size={18}/><div><b>{chat.title||'Untitled chat'}</b><small>{agentName(chat)} · {chat.status}</small></div><span>›</span></button>{:else}<p class="muted">No archived chats.</p>{/each}
      {:else if menu==='terminals'}<p class="view-hint">Terminal access is view-only on mobile.</p>{#each terminals as terminal}<button class="chat" onclick={()=>selectTerminal(terminal.id)}><Monitor size={18}/><div><b>{terminal.title}</b><small>{terminal.status}{terminal.exitCode===null?'':` · exit ${terminal.exitCode}`}</small></div><span>›</span></button>{:else}<p class="muted">No terminals are available.</p>{/each}
      {:else if menu==='preferences'}<MobilePreferences settings={snapshot.settings} workspaceView={mobileView}/>
      {:else if menu==='hosts'}<p class="view-hint">Host settings are read-only on mobile. Change them on desktop.</p>{#each snapshot.hosts as host}<section class="read-panel"><b>{host.name}</b><p>{host.kind==='local'?'Local desktop':`${host.user ? `${host.user}@` : ''}${host.address}${host.port ? `:${host.port}` : ''}`}</p><small>{host.defaultCwd || 'No default folder'}</small></section>{:else}<p class="muted">No hosts configured.</p>{/each}
      {:else}<p class="view-hint">Agent directory is read-only on mobile. Edit agents on desktop.</p>{#each snapshot.agents as agent}<section class="read-panel agent-directory"><span class="agent-mark" style={`background:${agent.color||'var(--accent)'}`}>{#if agent.avatar}<img src={agent.avatar} alt="" />{:else}<ProviderIcon provider={agent.provider} model={agent.model || agent.acp?.command} size={14}/>{/if}</span><div><b>{agent.name}</b><small>{agent.provider}{agent.model?` · ${agent.model}`:''}</small><p>{agent.description || 'No description.'}</p></div></section>{:else}<p class="muted">No agents configured.</p>{/each}{/if}
    </main>
    <nav class="bottom-nav" aria-label="Workspace navigation"><button class:active={menu==='workspace'} onclick={()=>menu='workspace'}><LayoutList size={19}/><span>Workspace</span></button><button class:active={menu==='archived'} onclick={()=>menu='archived'}><Archive size={19}/><span>Archived</span></button><button class:active={menu==='terminals'} onclick={openTerminals}><Monitor size={19}/><span>Terminals</span></button><button class:active={menu==='preferences'} onclick={()=>menu='preferences'}><ListFilter size={19}/><span>Preferences</span></button><button class:active={menu==='hosts'} onclick={()=>menu='hosts'}><Monitor size={19}/><span>Hosts</span></button><button class:active={menu==='agents'} onclick={()=>menu='agents'}><Bot size={19}/><span>Directory</span></button></nav>
  {/if}
  </div>
  {#if reconnecting}<div class="connection-recovery-spinner" data-testid="connection-recovery-spinner" role="status" aria-live="polite"><span class="connection-spinner" aria-hidden="true"></span></div>{/if}
</div>
<style>
.brand-name{letter-spacing:.24em;font-weight:700}.pin-fallback{width:100%;margin-top:20px}.pin-fallback summary{cursor:pointer;color:var(--soft);font-size:13px}.pin-fallback label{margin-top:16px}.saved-pairing{color:var(--soft)}.forget{opacity:.65}

  :global(body){margin:0;overflow:hidden}.mobile{--bg:#171916;--panel:#20241e;--line:#343932;--ink:#eeeee9;--muted:#929d90;--soft:#aab3a6;font-family:'IBM Plex Sans',system-ui,sans-serif;color:var(--ink);background:var(--bg);width:calc(100vw * var(--viewer-interface-scale-inverse,1));height:calc(100dvh * var(--viewer-interface-scale-inverse,1));transform:scale(var(--viewer-interface-scale,1));transform-origin:top left;display:flex;flex-direction:column;overflow-y:auto;padding-top:env(safe-area-inset-top);box-sizing:border-box}.mobile.light{--bg:#f7f5ef;--panel:#fffdf8;--line:#d8d5ca;--ink:#252921;--muted:#6a7168;--soft:#525a50}.mobile *{box-sizing:border-box}.verification{font:600 36px monospace;letter-spacing:5px;text-align:center;padding:18px}input[inputmode="numeric"]{font-size:24px;letter-spacing:3px}header{display:flex;align-items:center;gap:10px;padding:12px 16px;border-bottom:1px solid var(--line);flex:none;background:var(--panel)}header>div{flex:1;min-width:0}header strong{display:block;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;font-size:18px}small{display:block;font-size:12px;color:var(--muted)}header small{display:flex;gap:6px;align-items:center;margin-top:3px}header small span,.dot{width:7px;height:7px;border-radius:50%;background:#858b7f;display:inline-block}.online,.running{background:var(--accent)!important}button,textarea,input{font:inherit;color:inherit}button{border:0;background:transparent;cursor:pointer;min-height:44px;min-width:44px}button:disabled{opacity:.4}main{overflow:auto;flex:1;min-height:0;padding:20px}.pair{display:flex;flex-direction:column;justify-content:center;gap:18px;max-width:500px;width:100%;margin:auto}.mark,.working{color:var(--accent)}h1{font-size:30px;line-height:1.12;letter-spacing:-1px;margin:0}h2{font-size:15px;text-transform:uppercase;letter-spacing:.08em;color:var(--muted);margin:26px 0 8px}p{line-height:1.55;margin:0;color:var(--soft)}.primary,.secondary{border-radius:14px;padding:14px;display:flex;align-items:center;justify-content:center;gap:10px}.primary{background:var(--accent);color:var(--on-accent)}.secondary{border:1px solid var(--line)}label{font-size:13px;color:var(--soft)}textarea,input{display:block;width:100%;background:var(--panel);border:1px solid var(--line);border-radius:12px;padding:12px;resize:none;margin-top:8px}.workspace-summary{display:flex;justify-content:space-between;gap:12px;align-items:start}.workspace-summary .muted{margin-top:6px}.view-button{border:1px solid var(--line);border-radius:12px;display:flex;gap:5px;align-items:center;padding:0 10px;text-transform:capitalize;font-size:12px}.view-tabs{display:grid;grid-template-columns:repeat(3,1fr);gap:4px;background:var(--panel);padding:4px;border-radius:13px;margin:22px 0 8px}.view-tabs button{min-width:0;min-height:38px;border-radius:9px;font-size:12px;display:flex;gap:5px;justify-content:center;align-items:center}.view-tabs button.active{background:var(--accent);color:var(--on-accent)}.section-head{width:100%;display:flex;align-items:center;gap:10px;text-align:left;padding:10px 0;border-bottom:1px solid var(--line)}.section-head>span:nth-child(2){flex:1;font-weight:650}.section-head small{margin-top:3px}.agent-mark,.project-mark{width:24px;height:24px;display:grid;place-items:center;border-radius:8px;color:white}.count{font-size:12px;color:var(--muted)}.chat{display:flex;align-items:center;gap:12px;text-align:left;width:100%;padding:12px 2px;border-bottom:1px solid var(--line)}.chat>div{flex:1;min-width:0}.chat b{display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.chat small{margin-top:4px}.indent{padding:10px 0 10px 34px;font-size:13px}.view-hint{font-size:13px;margin:4px 0 12px}.channels{padding-bottom:8px}.thread{padding-bottom:24px}.thread article{margin-bottom:20px;padding:12px 0}.thread article.user{background:color-mix(in srgb,var(--accent) 20%,var(--panel));padding:14px;border-radius:16px}.thread article.final-answer{width:fit-content;padding:12px 14px;border:1px solid color-mix(in srgb,#4f9d69 18%,var(--line));border-radius:14px 14px 14px 4px;background:color-mix(in srgb,#4f9d69 7%,var(--panel))}.thread .message-body{white-space:pre-wrap;overflow-wrap:anywhere;line-height:1.6}.capability-note{display:flex;gap:9px;align-items:flex-start;padding:11px 12px;margin-bottom:16px;background:var(--panel);border:1px solid var(--line);border-radius:12px;color:var(--soft);font-size:13px;line-height:1.4}.capability-note :global(svg){flex:none;color:var(--accent)}.read-only{padding:13px 18px calc(13px + env(safe-area-inset-bottom));border-top:1px solid var(--line);background:var(--panel);font-size:13px;color:var(--muted)}.composer-glow{position:relative;height:44px;flex:none;overflow:hidden}
form{display:flex;align-items:end;gap:8px;padding:12px 16px calc(12px + env(safe-area-inset-bottom));border-top:1px solid var(--line);background:var(--panel)}form textarea{flex:1;min-width:0;margin:0;overflow:hidden}.send{background:var(--accent);color:var(--on-accent);border-radius:50%;height:44px}.terminal{padding:16px}.terminal pre{margin:0;background:#10120f;color:#dfe7dc;padding:14px;border-radius:12px;white-space:pre-wrap;overflow-wrap:anywhere;font:12px/1.55 'IBM Plex Mono',ui-monospace,monospace;min-height:180px}.error{padding:12px 20px;background:#492e2a;color:#ffd7ca;font-size:13px}.muted{color:var(--muted)}.bottom-nav{display:grid;grid-template-columns:repeat(3,1fr);padding:7px 10px calc(7px + env(safe-area-inset-bottom));border-top:1px solid var(--line);background:var(--panel)}.bottom-nav button{min-width:0;min-height:48px;border-radius:11px;display:grid;place-items:center;gap:1px;color:var(--muted);font-size:11px}.bottom-nav button.active{color:var(--on-accent);background:var(--accent)}@media (max-width:340px){main{padding:16px}.view-tabs button{font-size:11px;gap:2px}.workspace-summary h1{font-size:27px}}
.agent-mark img{width:100%;height:100%;object-fit:cover;border-radius:inherit}
.cancellation-event{display:flex;align-items:center;gap:8px;margin:4px 0 14px;padding:5px 2px;color:var(--muted)}.cancellation-event :global(.message-meta){flex:1;min-width:0;margin:0}.cancellation-event :global(.message-meta time){margin-left:auto}.cancellation-event :global(svg){flex:none;color:#b56a54}
.context-cleared-event{display:flex;align-items:center;gap:8px;margin:5px 0 16px;color:var(--muted);font:9px monospace;text-transform:uppercase;letter-spacing:.04em;white-space:nowrap}.context-cleared-event span{height:1px;flex:1;background:var(--line)}
.load-earlier{width:100%;min-width:0;min-height:38px;border:1px solid var(--line);border-radius:10px;color:var(--soft);font-size:12px;margin-bottom:16px}.history-error{color:#d88976;font-size:12px;margin-bottom:16px}.read-panel{padding:14px;margin:10px 0;background:var(--panel);border:1px solid var(--line);border-radius:12px}.read-panel p{font-size:13px;margin-top:6px}.agent-directory{display:flex;gap:12px}.agent-directory>div{min-width:0}.agent-directory .agent-mark{flex:none}
.thread article.user,.thread article.final-answer{padding:0;border:0;border-radius:0;background:transparent}
.thread article>.message-bubble{padding:12px 0}
.thread article.user>.message-bubble{padding:14px;border-radius:16px;background:color-mix(in srgb,var(--accent) 20%,var(--panel))}
  .thread article.final-answer>.message-bubble{padding:12px 14px;border:1px solid color-mix(in srgb,#4f9d69 18%,var(--line));border-radius:14px 14px 14px 4px;background:color-mix(in srgb,#4f9d69 7%,var(--panel))}
  .connection-recovery-spinner{position:fixed;right:16px;bottom:calc(16px + env(safe-area-inset-bottom));z-index:10000;pointer-events:none;color:var(--ink)}.connection-spinner{display:block;width:18px;height:18px;border:2px solid color-mix(in srgb,currentColor 25%,transparent);border-top-color:currentColor;border-radius:50%;animation:connection-spin .8s linear infinite}@keyframes connection-spin{to{transform:rotate(360deg)}}
  header>button[aria-label="Open chats menu"]{color:#fff}
</style>
