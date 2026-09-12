<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { ArchiveX, ShieldCheck } from '@lucide/svelte';
  import QRCode from 'qrcode';
  import { getBridge } from '$lib/bridge';
  import { createResumableDesktopSession, type ResumableDesktopSession } from '$lib/controller/resumable-session';
  import type { RemoteConnectionState } from '$lib/controller/remote-client';
  import { clearDesktopPairing, deviceExpiresAt, isRememberedController, MAX_INACTIVITY_DAYS, newDesktopPairing, saveDesktopPairing, loadDesktopPairing, type DesktopPairingRecord, type RememberedController } from '$lib/controller/pairing-store';
  import { DEFAULT_RELAY, registerPairingCode, revokePairingCode, type PairingRegistration } from '$lib/controller/pairing-code';
  import { remoteControlTarget } from '$lib/workspace-panels';

  let { open = $bindable(false) }: { open?: boolean } = $props();
  let relay=$state(DEFAULT_RELAY), qr=$state(''), status=$state('off'), error=$state(''), verificationCode=$state(''), creating=$state(false), now=$state(Date.now()), policyInput=$state('');
  let registration=$state<PairingRegistration|null>(null), session=$state<ResumableDesktopSession|null>(null), unsubscribe:(()=>void)|undefined;
  let connectionError='',pairingError='';
  let record=$state.raw<DesktopPairingRecord|null>(null), generation=0, loaded=$state(false), saving=$state(false), writesPending=0, revision=0;
  const timer=setInterval(()=>now=Date.now(),1000);
  const key=$derived(registration && registration.expiresAt>now ? registration.code.match(/.{3}/g)?.join(' ') : '');
  const devices=$derived(record?.devices??[]);
  function portal(node:HTMLElement,target:HTMLElement|null){
    const fallback=node.parentElement!;
    let current:HTMLElement|null=null,observer:MutationObserver|undefined;
    function move(next:HTMLElement|null){
      observer?.disconnect();current=next?.isConnected?next:null;
      (current??fallback).appendChild(node);
      if(current){
        observer=new MutationObserver(()=>{if(current&&!current.isConnected)move(null);});
        observer.observe(document.documentElement,{childList:true,subtree:true});
      }
    }
    move(target);
    return {update:move,destroy:()=>observer?.disconnect()};
  }
  function statusLabel(value:string){return ({connecting:error?'Reconnecting to the relay…':'Connecting to the relay…',waiting_for_peer:'Ready for a phone',pending:'Waiting for phone approval',connected:'Phone connected',closed:'Listener stopped',rejected:'Phone pairing rejected',error:'Remote access needs attention'} as Record<string,string>)[value]??value.replaceAll('_',' ');}
  function clearRegistration(){const old=registration;registration=null;if(old)void revokePairingCode(relay,old).catch(()=>{});}
  function clone(source:DesktopPairingRecord):DesktopPairingRecord{return {version:1,relayUrl:source.relayUrl,enabled:source.enabled,inactivityDays:source.inactivityDays,identity:{room:source.identity.room,keyPair:source.identity.keyPair},devices:source.devices.map(device=>({...device}))};}
  function controllerBridge(){const bridge=getBridge();return {...bridge,sendMessage:(taskId:string,text:string)=>bridge.sendMessage(taskId,text)};}
  async function persist(next:DesktopPairingRecord,current=generation){
    if(current!==generation)return;
    // Install synchronously: an older completed write must never restore revoked trust.
    record=next;const writeRevision=++revision;writesPending++;saving=true;
    try{await saveDesktopPairing(next);}
    catch(reason){
      if(current===generation){
        error='Could not save remote access policy: '+String(reason);
        if(writeRevision===revision&&record)record={...record,enabled:false};
      }
      throw reason;
    }finally{writesPending--;saving=writesPending>0;}
  }
  function trusted(publicKey:string){return !!record&&isRememberedController(record,publicKey,Date.now());}
  function access(publicKey:string,current:number){
    if(current!==generation||!record||!trusted(publicKey))return;
    const next=clone(record),device=next.devices.find(item=>item.publicKey===publicKey);
    if(!device)return;device.lastAccessAt=Date.now();
    void persist(next,current).catch(()=>{});
  }
  function connectionChanged(value:RemoteConnectionState,current:number){
    if(current!==generation)return;
    status=value.status;verificationCode=value.verificationCode??'';
    if(value.error){connectionError=value.error;error=value.error;}
    else if(['waiting_for_peer','connected'].includes(value.status)&&error===connectionError){error='';connectionError='';}
    if(['pending','connected'].includes(value.status)&&error===pairingError){error='';pairingError='';}
    if(['pending','connected','closed','error','rejected'].includes(value.status))clearRegistration();
  }
  async function startListener(existing?:DesktopPairingRecord){
    if(creating)return;unsubscribe?.();session?.close();session=null;const current=++generation;creating=true;error='';
    try{if(!getBridge().available)throw new Error('Open Remote control inside the Monitter desktop app.');const nextRecord=existing??await newDesktopPairing(relay);nextRecord.enabled=true;nextRecord.relayUrl=relay;await persist(nextRecord,current);if(current!==generation)return;relay=nextRecord.relayUrl;policyInput=nextRecord.inactivityDays===null?'':String(nextRecord.inactivityDays);const next=await createResumableDesktopSession({relayUrl:nextRecord.relayUrl,bridge:controllerBridge(),identity:nextRecord.identity,isTrustedController:publicKey=>current===generation&&trusted(publicKey),onControllerAccess:publicKey=>access(publicKey,current)});if(current!==generation){next.close();return;}session=next;unsubscribe=next.subscribe(value=>connectionChanged(value,current));status=next.getStatus();}
    catch(reason){if(current===generation){error=String(reason);status='error';}}finally{if(current===generation)creating=false;}
  }
  async function createPairingCode(){
    if(creating||!session||session.getStatus()!=='waiting_for_peer')return;
    const current=generation,ownedSession=session;creating=true;error='';pairingError='';
    try{
      const invitation=ownedSession.invitation;
      const image=await QRCode.toDataURL(invitation,{width:280,margin:2});
      if(current!==generation||ownedSession.getStatus()!=='waiting_for_peer')return;
      qr=image;
      const created=await registerPairingCode(invitation);
      if(current!==generation||ownedSession.getStatus()!=='waiting_for_peer'){void revokePairingCode(relay,created).catch(()=>{});return;}
      registration=created;
    }catch(reason){if(current===generation&&ownedSession.getStatus()==='waiting_for_peer'){pairingError=reason instanceof Error?reason.message:String(reason);error=pairingError;}}
    finally{if(current===generation)creating=false;}
  }
  function closeRuntime(){generation++;unsubscribe?.();unsubscribe=undefined;session?.close();session=null;clearRegistration();qr='';status='off';verificationCode='';creating=false;}
  async function approve(){if(!session||!record||saving)return;const publicKey=session.getPeerControllerPublicKey(),peer=session.getPeer();if(peer!==null){error='This is a shared visitor connection. Use sharing controls instead of remembering it as a phone.';await session.reject().catch(()=>{});return;}if(!publicKey){error='The phone identity was unavailable. Reject it and create a fresh pairing code.';return;}const current=generation,next=clone(record),timestamp=Date.now(),existing=next.devices.find(device=>device.publicKey===publicKey);if(existing){existing.approvedAt=timestamp;existing.lastAccessAt=timestamp;}else{if(next.devices.length>=32){error='You can remember up to 32 phones. Revoke one before approving another.';return;}next.devices.push({publicKey,name:'Phone '+(next.devices.length+1),approvedAt:timestamp,lastAccessAt:timestamp});}try{await persist(next,current);if(current!==generation||!trusted(publicKey))return;await session?.approve();}catch(reason){if(current===generation)error='Phone was not approved: '+String(reason);}}
  async function reject(){try{await session?.reject();}catch(reason){error=String(reason);}}
  async function revokeDevice(publicKey:string){if(!record)return;const current=generation,next=clone(record);next.devices=next.devices.filter(device=>device.publicKey!==publicKey);record=next;const active=session?.getPeerControllerPublicKey()===publicKey?session:null;const rejection=active?.reject().catch(()=>{});try{await persist(next,current);await rejection;if(current!==generation)return;if(active===session)await active?.reconnectNow();}catch{}}
  async function revokeAll(){
    const current=++generation,previous=session;
    clearRegistration();unsubscribe?.();unsubscribe=undefined;session=null;record=null;revision++;
    status='off';qr='';creating=false;verificationCode='';
    // Queue deletion immediately after prior saves; old listeners cannot enqueue more writes.
    const deletion=clearDesktopPairing();
    void previous?.reject().catch(()=>{}).finally(()=>previous.close());
    try{await deletion;}catch(reason){if(current===generation)error='Could not clear saved remote access: '+String(reason);}
  }
  async function savePolicy(){if(!record||saving)return;const trimmed=policyInput.trim(),days=trimmed===''?null:Number(trimmed);if(!(days===null||(Number.isInteger(days)&&days>=1&&days<=MAX_INACTIVITY_DAYS))){error='Enter whole days from 1 to '+MAX_INACTIVITY_DAYS+', or leave this blank for indefinite access.';return;}const current=generation,next=clone(record);next.inactivityDays=days;try{await persist(next,current);if(current!==generation)return;const peer=session?.getPeerControllerPublicKey();if(peer&&status==='connected'&&!trusted(peer)){await session?.reject().catch(()=>{});await session?.reconnectNow();}}catch{}}
  function expiry(device:RememberedController){return deviceExpiresAt(device,record?.inactivityDays??null);}
  function deviceState(device:RememberedController){const expires=expiry(device);if(expires!==null&&expires<=now)return'expired';return session?.getPeerControllerPublicKey()===device.publicKey&&status==='connected'?'connected':'offline';}
  function stamp(value:number){return new Intl.DateTimeFormat(undefined,{dateStyle:'medium',timeStyle:'short'}).format(value);}
  onMount(()=>{
    const current=generation;
    void(async()=>{
      try{
        const saved=await loadDesktopPairing();if(current!==generation||!saved)return;
        record=saved;relay=saved.relayUrl;policyInput=saved.inactivityDays===null?'':String(saved.inactivityDays);
        if(saved.enabled)await startListener(saved);
      }catch(reason){if(current===generation)error='Could not restore remote access: '+String(reason);}
      finally{loaded=true;}
    })();
  });
  onDestroy(()=>{clearInterval(timer);closeRuntime();});
</script>
{#if open}<div class="remote-control-fallback" aria-hidden="true"><div class="remote-panel" use:portal={$remoteControlTarget} role="region" aria-label="Remote control">
<header><h2>Remote control</h2></header>
<p>Paired phones can view and control this desktop through an encrypted relay.</p>
<label>Relay address<input bind:value={relay} placeholder="wss://relay.example.com" disabled={!!session||creating}/></label>
<section class="policy"><h3>Remembered phones</h3><label>Inactivity limit (days)<input inputmode="numeric" bind:value={policyInput} placeholder="Indefinite" disabled={!record||saving}/></label><small>Blank means access remains until you revoke it. Each time a phone connects or uses the desktop, its full day limit starts again.</small><button onclick={savePolicy} disabled={!record||saving}>Save policy</button></section>
{#if !session}<button class="primary" disabled={creating||!loaded} onclick={()=>startListener(record??undefined)}>{creating?'Starting…':record?.enabled?'Restart remote listener':'Enable remote access'}</button>
{:else}
  <p role="status">{statusLabel(status)}</p>
  {#if status==='waiting_for_peer'}
    {#if qr}<img src={qr} alt="Mobile pairing QR code"/>{/if}
    {#if key}
      <div>One-time device key<strong class="device-key">{key}</strong></div>
      <small>Expires in {Math.max(0,Math.ceil(((registration?.expiresAt??0)-now)/1000))} seconds. One use only.</small>
    {/if}
    {#if !registration}<button class="primary" disabled={creating} onclick={createPairingCode}>{creating?'Creating…':qr?'Retry pairing code':'Create pairing code'}</button>{/if}
  {/if}
  {#if status==='pending'}
    <p>Check that your phone shows this same verification number:</p>
    <strong class="device-key">{verificationCode}</strong>
    <button class="primary" disabled={saving} onclick={approve}>Numbers match — approve phone</button>
    <button onclick={reject}>Reject</button>
  {/if}
  <button onclick={closeRuntime}>Stop listener</button>
{/if}
<section class="devices"><h3>Saved phones</h3>{#each devices as device (device.publicKey)}<article><div><strong>{device.name}</strong><small class:connected={deviceState(device)==='connected'} class:expired={deviceState(device)==='expired'}>{deviceState(device)} · Last access {stamp(device.lastAccessAt)}{#if expiry(device)!==null} · Expires {stamp(expiry(device)!)}{/if}</small></div><button class="danger" aria-label={'Revoke '+device.name} onclick={()=>revokeDevice(device.publicKey)}><ArchiveX size={15}/>Revoke</button></article>{:else}<p class="empty">No phones are remembered yet.</p>{/each}</section>
{#if devices.length||record}<button class="danger revoke-all" onclick={revokeAll}><ShieldCheck size={15}/>Revoke all saved access</button>{/if}
{#if error}<p role="alert">{error}</p>{/if}
<small>One-time pairing codes expire after five minutes. Stopping the listener does not forget saved phones.</small>
</div></div>{/if}
<style>
.remote-control-fallback{display:none}.device-key{display:block;font:600 28px var(--mono,monospace);letter-spacing:3px;margin:12px 0;color:var(--accent-ink,var(--accent))}.remote-panel{width:100%;box-sizing:border-box;min-width:0;padding-top:4px;color:var(--ink);font:14px var(--interface-font,'IBM Plex Sans',sans-serif)}.remote-panel header,.devices article{display:flex;align-items:center;justify-content:space-between;gap:12px}.remote-panel h2{font-size:20px;margin:0}.remote-panel h3{font-size:14px;margin:0 0 8px}.remote-panel p{line-height:1.5}.remote-panel label{display:block;margin:12px 0}.remote-panel input{box-sizing:border-box;width:100%;margin-top:6px;border:1px solid var(--line);background:var(--paper);color:inherit;border-radius:8px;padding:10px;font:inherit}.remote-panel button{display:inline-flex;align-items:center;gap:6px;padding:10px;border:0;border-radius:8px;background:var(--soft);color:inherit;cursor:pointer;margin:4px}.remote-panel button:disabled{opacity:.5;cursor:default}.remote-panel button.primary{background:var(--accent);color:var(--on-accent)}.remote-panel img{display:block;width:min(280px,100%);height:auto;margin:12px auto;border-radius:10px}.remote-panel small{display:block;color:var(--muted);line-height:1.5;margin-top:5px}.policy,.devices{border-top:1px solid var(--line);margin-top:16px;padding-top:14px}.devices article{padding:10px 0;border-bottom:1px solid var(--line)}.devices article strong{display:block}.connected{color:var(--accent-ink)!important}.expired{color:#a13a2a!important}:global(:root[data-theme="dark"]) .expired{color:#f0a48e!important}.danger{background:#5a302b!important;color:#fff!important}.revoke-all{margin-top:14px}.empty{color:var(--muted)}
</style>
