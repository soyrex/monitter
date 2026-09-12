<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { ArrowLeft, ArrowUp, Square, QrCode, Unplug, RefreshCw } from '@lucide/svelte';
  import type { Snapshot } from '$lib/types';
  import { createMobileSession } from '$lib/controller/remote-client';
  import { redeemPairingCode } from '$lib/controller/pairing-code';
  import MessageMeta from '$lib/components/MessageMeta.svelte';
  let deviceKey=$state(''), verificationCode=$state(''), connecting=$state(false);
  let connectionGeneration=0;
  let status=$state('disconnected'), error=$state('');
  let snapshot=$state<Snapshot|null>(null), selected=$state<string|null>(null), text=$state(''), busy=$state(false);
  let session=$state<Awaited<ReturnType<typeof createMobileSession>>|null>(null);
  let unsubscribe:(()=>void)|undefined;
  let refreshing=false;
  let chatDrafts=$state<Record<string,string>>({});
  function selectChat(id:string|null){if(selected)chatDrafts[selected]=text;selected=id;text=id?chatDrafts[id]??'':'';}
  let thread=$state<HTMLElement>();
  let atBottom=$state(true);
  $effect(()=>{selected;void tick().then(()=>{if(thread){thread.scrollTop=thread.scrollHeight;atBottom=true;}});});
  const task=$derived(snapshot?.tasks.find(t=>t.id===selected));
  const messages=$derived(snapshot?.messages.filter(m=>m.taskId===selected)??[]);
  async function refresh(){if(!session||refreshing)return; const current=session;refreshing=true;try{const next=await current.getSnapshot();if(current!==session)return;snapshot=next;error='';if(atBottom){await tick();if(thread)thread.scrollTop=thread.scrollHeight;}}catch(e){if(current===session)error=String(e);}finally{refreshing=false;}}
  async function connect(scannedInvitation?:string){
    if(connecting)return;
    const generation=++connectionGeneration;connecting=true;error='';verificationCode='';
    session?.close();unsubscribe?.();session=null;snapshot=null;status='connecting';
    try {
      const invitation=scannedInvitation??await redeemPairingCode(deviceKey);
      if(generation!==connectionGeneration)return;
      const next=await createMobileSession(invitation);
      if(generation!==connectionGeneration){next.close();return;}
      session=next;
      unsubscribe=next.subscribe(value=>{status=value.status;verificationCode=value.verificationCode??'';if(value.error)error=value.error;if(status==='connected')void refresh();});
    }catch(e){if(generation===connectionGeneration){error=String(e);status='error';}}
    finally{if(generation===connectionGeneration)connecting=false;}
  }
  function disconnect(){connectionGeneration++;connecting=false;unsubscribe?.();session?.close();session=null;snapshot=null;selected=null;text='';chatDrafts={};status='disconnected';deviceKey='';verificationCode='';}
  async function send(){if(!session||!selected||!text.trim()||busy)return;const body=text,taskId=selected,current=session;busy=true;try{const next=await current.sendMessage(taskId,body);if(current!==session)return;snapshot=next;if(selected===taskId){if(text===body)text='';}else if(chatDrafts[taskId]===body)chatDrafts[taskId]='';}catch(e){error=String(e);}finally{busy=false;}}
  async function stop(){if(!session||!selected)return;try{snapshot=await session.cancelTask(selected);}catch(e){error=String(e);}}
  function scan(){
    error='';
    const android=(window as any).monitterAndroid;
    const ios=(window as any).webkit?.messageHandlers?.scanQR;
    if(android)android.postMessage('scanQR');else if(ios)ios.postMessage(null);else error='Enter the nine-digit device key shown in desktop Remote control below.';
  }
  onMount(()=>{
    const qr=(event:Event)=>{const value=(event as CustomEvent<string>).detail;if(typeof value==='string'&&value.length<=4096)void connect(value);};
    const qrError=(event:Event)=>{error=String((event as CustomEvent<string>).detail||'Scanner unavailable. Enter the device key instead.');};
    window.addEventListener('monitter:qr',qr);window.addEventListener('monitter:qr-error',qrError);
    const timer=setInterval(()=>{if(session&&status==='connected'&&!document.hidden)void refresh();},2000);
    return()=>{clearInterval(timer);window.removeEventListener('monitter:qr',qr);window.removeEventListener('monitter:qr-error',qrError);connectionGeneration++;session?.close();unsubscribe?.();};
  });
</script>
<svelte:head><title>Monitter Remote</title><meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover" /></svelte:head>
<div class="mobile" style:--interface-font-ratio={(snapshot?.settings.interfaceFontSize ?? 14) / 14}>
<header>{#if selected}<button aria-label="Back to chats" onclick={()=>selectChat(null)}><ArrowLeft size={22}/></button>{/if}<div><strong>{task?.title??'monitter'}</strong><small><span class:online={status==='connected'}></span>{status.replaceAll('_',' ')}</small></div>{#if session}<button aria-label="Refresh" onclick={refresh}><RefreshCw size={20}/></button><button aria-label="Disconnect" onclick={disconnect}><Unplug size={20}/></button>{/if}</header>
{#if error}<div class="error" role="alert">{error}</div>{/if}
{#if !snapshot}
<main class="pair"><div class="mark"><QrCode size={38}/></div><h1>Your desktop.<br/>In your pocket.</h1>
{#if status==='awaiting_approval'}
<p>Compare this number with your desktop, then approve the connection there.</p><strong class="verification">{verificationCode}</strong>
{:else if status==='waiting_for_peer' || connecting}
<p>{connecting?'Connecting…':'Waiting for your desktop. Keep Remote control open there. If this code is old, create a fresh one.'}</p>
{:else}
<p>Follow your agents and keep the conversation moving. Work stays on your desktop.</p>
<button class="primary" onclick={scan}><QrCode size={20}/>Scan desktop code</button>
<label>Or enter a one-time device key<input aria-label="Device key" inputmode="numeric" autocomplete="one-time-code" maxlength="11" bind:value={deviceKey} placeholder="123 456 789" /></label>
<button class="secondary" disabled={!/^\d{9}$/.test(deviceKey.replace(/[\s-]/g,''))||connecting} onclick={()=>connect()}>Connect to desktop</button>
<small>Device keys expire after five minutes and work once. Compare the verification numbers before approving on desktop.</small>
{/if}
{#if session && status!=='connected'}<button class="secondary" onclick={disconnect}>Start again</button>{/if}
</main>
{:else if !selected}
<main class="chats"><h1>Your workspace</h1><p class="muted">{snapshot.tasks.filter(t=>t.status==='running').length} running · {snapshot.agents.length} agents</p>{#each snapshot.agents as agent}<section><h2>{agent.name}<small>{agent.provider}</small></h2>{#each snapshot.tasks.filter(t=>t.agentId===agent.id&&!t.archived&&!t.channelId).sort((a,b)=>b.updatedAt-a.updatedAt) as chat}<button class="chat" onclick={()=>selectChat(chat.id)}><span class="dot" class:running={chat.status==='running'}></span><div><b>{chat.title||'Untitled chat'}</b><small>{chat.status}</small></div><span>›</span></button>{:else}<p class="muted">No open chats</p>{/each}</section>{/each}</main>
{:else}
<main class="thread" bind:this={thread} onscroll={()=>{if(thread)atBottom=thread.scrollHeight-thread.scrollTop-thread.clientHeight<60;}}>{#each messages as message}<article class:user={message.role==='user'}><MessageMeta name={message.role==='user'?'You':snapshot.agents.find(a=>a.id===task?.agentId)?.name??message.role} createdAt={message.createdAt}/><div>{message.text}</div></article>{/each}{#if task?.status==='running'}<p class="working">Agent is working…</p>{/if}</main>
<form onsubmit={event=>{event.preventDefault();void send();}}><textarea aria-label="Message" placeholder="Message your agent…" bind:value={text} rows="2"></textarea>{#if task?.status==='running'}<button type="button" aria-label="Stop agent" onclick={stop}><Square size={20}/></button>{/if}<button class="send" aria-label="Send message" disabled={busy||!text.trim()||status!=='connected'}><ArrowUp size={24}/></button></form>
{/if}
</div>
<style>
.verification{font:600 36px monospace;letter-spacing:5px;text-align:center;padding:18px}input[inputmode="numeric"]{font-size:24px;letter-spacing:3px}
:global(body){margin:0}.mobile{font-family:'IBM Plex Sans',system-ui,sans-serif;color:#eeeee9;background:#171916;height:100dvh;display:flex;flex-direction:column;padding-top:env(safe-area-inset-top);box-sizing:border-box}.mobile *{box-sizing:border-box}header{display:flex;align-items:center;gap:14px;padding:16px 20px;border-bottom:1px solid #343932;flex:none}header>div{flex:1;min-width:0}header strong{display:block;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;font-size:21px}small{display:block;font-size:12px;color:#929d90}header small{display:flex;gap:6px;align-items:center;margin-top:3px}header small span,.dot{width:7px;height:7px;border-radius:50%;background:#858b7f;display:inline-block}.online,.running{background:#62c694!important}button,textarea,input{font:inherit;color:inherit}button{border:0;background:transparent;cursor:pointer;min-height:44px;min-width:44px}button:disabled{opacity:.4}main{overflow:auto;flex:1;min-height:0;padding:24px}.pair{display:flex;flex-direction:column;justify-content:center;gap:18px;max-width:500px;width:100%;margin:auto}.mark{color:#66c698}h1{font-size:32px;line-height:1.12;letter-spacing:-1px;margin:0}p{line-height:1.6;margin:0;color:#aab3a6}.primary,.secondary{border-radius:14px;padding:14px;display:flex;align-items:center;justify-content:center;gap:10px}.primary{background:#73ce9f;color:#13261b}.secondary{border:1px solid #475144}label{font-size:13px;color:#aab3a6}textarea,input{display:block;width:100%;background:#22271f;border:1px solid #40493b;border-radius:12px;padding:12px;resize:none;margin-top:8px}section{margin-top:28px}h2{font-size:18px;display:flex;align-items:center;gap:8px}.chat{display:flex;align-items:center;gap:12px;text-align:left;width:100%;padding:14px 0;border-bottom:1px solid #2d332a}.chat>div{flex:1;min-width:0}.chat b{display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.chat small{margin-top:4px}.thread article{margin-bottom:24px;padding:12px 0}.thread article.user{background:#253d2e;padding:16px;border-radius:16px}.thread article>div{white-space:pre-wrap;overflow-wrap:anywhere;line-height:1.6;margin-top:8px}.working{color:#73ce9f}form{display:flex;align-items:end;gap:8px;padding:12px 16px calc(12px + env(safe-area-inset-bottom));border-top:1px solid #343932}form textarea{flex:1;min-width:0;margin:0}.send{background:#73ce9f;color:#13261b;border-radius:50%;height:44px}.error{padding:12px 20px;background:#492e2a;color:#ffd7ca;font-size:13px}.muted{color:#929d90}
</style>
