<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import ArchiveX from "@lucide/svelte/icons/archive-x";
  import ShieldCheck from "@lucide/svelte/icons/shield-check";
  import QRCode from 'qrcode';
  import { getBridge } from '$lib/bridge';
  import { bytesToBase64url, randomBytes, exportPairingPublicKey } from '$lib/controller/secure-session';
  import { pairingHost, pairingHostBridge } from '$lib/controller/gpui-pairing-host';
  import { invokeCommand } from '$lib/command-invoke';
  import { createResumableDesktopSession, type ResumableDesktopSession } from '$lib/controller/resumable-session';
  import { watchReconnectEvents } from '$lib/controller/reconnect-events';
  import type { DesktopBridge, RemoteConnectionState } from '$lib/controller/remote-client';
  import { clearDesktopPairing, deviceExpiresAt, isRememberedController, isRememberedOwnerWorkspace, MAX_INACTIVITY_DAYS, newDesktopPairing, saveDesktopPairing, loadDesktopPairing, type DesktopPairingRecord, type RememberedController } from '$lib/controller/pairing-store';
  import { DEFAULT_RELAY, registerPersistentPairingCode, revokePairingCode, type PairingRegistration } from '$lib/controller/pairing-code';
  import { remoteControlTarget } from '$lib/workspace-panels';
  import { applyOwnerAppearance, currentOwnerAppearance } from '$lib/owner-appearance';
  import type { JsonValue } from '$lib/types';

  let { open = $bindable(false), standalone = false }: { open?: boolean; standalone?: boolean } = $props();
  let revoking=$state(false);
  let relay=$state(DEFAULT_RELAY), qr=$state(''), status=$state('off'), error=$state(''), verificationCode=$state(''), creating=$state(false), now=$state(Date.now()), policyInput=$state('');
  let registration=$state<PairingRegistration|null>(null), session=$state<ResumableDesktopSession|null>(null), unsubscribe:(()=>void)|undefined, unsubscribeOwner:(()=>void)|undefined;
  let connectionError='',pairingError='';
  let record=$state.raw<DesktopPairingRecord|null>(null), generation=0, loaded=$state(false), saving=$state(false), writesPending=0, revision=0;
  const timer=setInterval(()=>now=Date.now(),1000);
  const key=$derived(registration && (registration.persistent || (registration.expiresAt??0)>now) ? registration.code.match(/.{3}/g)?.join(' ') : '');
  const devices=$derived(record?.devices??[]), ownerDevices=$derived(record?.ownerDevices??[]), ownerPending=$derived.by(()=>{now;return session?.getOwnerWorkspaceState()==='requested';});
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
  function clearRegistration(){const old=registration;registration=null;if(old&&!old.persistent)void revokePairingCode(relay,old).catch(()=>{});}
  function clone(source:DesktopPairingRecord):DesktopPairingRecord{return {version:2,relayUrl:source.relayUrl,enabled:source.enabled,inactivityDays:source.inactivityDays,identity:{room:source.identity.room,keyPair:source.identity.keyPair},devices:source.devices.map(device=>({...device})),ownerDevices:source.ownerDevices.map(device=>({...device})),...(source.pairingManagementToken?{pairingManagementToken:source.pairingManagementToken}:{}),...(source.pairingRegistration?{pairingRegistration:{...source.pairingRegistration}}:{})};}
  // General mobile control deliberately excludes visitor-only file upload.
  function controllerBridge():DesktopBridge{
    const native=pairingHostBridge();
    if(native)return native;
    const bridge=getBridge();
    return {
      getSnapshot:()=>bridge.getSnapshot(),
      sendMessage:(taskId,text,attachmentIds)=>bridge.sendMessage(taskId,text,attachmentIds),
      cancelTask:taskId=>bridge.cancelTask(taskId),
      resumeTask:taskId=>bridge.resumeTask(taskId),
      listTerminals:()=>bridge.listTerminals(),
      readTerminal:(id,afterSeq)=>bridge.readTerminal(id,afterSeq),
    };
  }
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
  function trustedOwner(publicKey:string){return !!record&&isRememberedOwnerWorkspace(record,publicKey,Date.now());}
  function access(publicKey:string,current:number){
    if(current!==generation||!record||!trusted(publicKey))return;
    const next=clone(record),device=next.devices.find(item=>item.publicKey===publicKey);
    if(!device)return;device.lastAccessAt=Date.now();
    void persist(next,current).catch(()=>{});
  }
  function accessOwner(publicKey:string,current:number){
    if(current!==generation||!record||!trustedOwner(publicKey))return;
    const next=clone(record),device=next.ownerDevices.find(item=>item.publicKey===publicKey);
    if(!device)return;device.lastAccessAt=Date.now();
    void persist(next,current).catch(()=>{});
  }
  function connectionChanged(value:RemoteConnectionState,current:number){
    if(current!==generation)return;
    status=value.status;verificationCode=value.verificationCode??'';
    if(value.error){connectionError=value.error;error=value.error;}
    else if(['waiting_for_peer','connected'].includes(value.status)&&error===connectionError){error='';connectionError='';}
    if(['pending','connected'].includes(value.status)&&error===pairingError){error='';pairingError='';}
    if(!registration?.persistent&&['pending','connected','closed','error','rejected'].includes(value.status))clearRegistration();
    if(value.status==='waiting_for_peer'&&!creating&&!registration)void createPairingCode();
  }
  async function startListener(existing?:DesktopPairingRecord){
    if(creating||revoking)return;unsubscribe?.();unsubscribeOwner?.();session?.close();session=null;const current=++generation;creating=true;error='';
    try{if(pairingHost()){try{await pairingHost()!.invoke('get_command_capabilities');}catch{throw new Error('Restart Monitter and its backend from the menu-bar server to activate phone pairing. Active agents can finish first.');}}if((existing?.pairingRegistration||existing?.pairingManagementToken)&&existing.relayUrl!==relay)throw new Error('Revoke this desktop PIN before changing its relay address.');if(!pairingHost()&&!getBridge().available)throw new Error('Open Remote control inside the Monitter desktop app.');const nextRecord=existing??await newDesktopPairing(relay);nextRecord.enabled=true;nextRecord.relayUrl=relay;await persist(nextRecord,current);if(current!==generation)return;relay=nextRecord.relayUrl;policyInput=nextRecord.inactivityDays===null?'':String(nextRecord.inactivityDays);const next=await createResumableDesktopSession({relayUrl:nextRecord.relayUrl,bridge:controllerBridge(),identity:nextRecord.identity,isTrustedController:publicKey=>current===generation&&trusted(publicKey),onControllerAccess:publicKey=>access(publicKey,current),isTrustedOwnerWorkspace:publicKey=>current===generation&&trustedOwner(publicKey),onOwnerWorkspaceAccess:publicKey=>accessOwner(publicKey,current),onOwnerInvoke:(command,args)=>{
      // Presentation state lives in the desktop renderer, not the Rust Snapshot.
      // These two tiny, validated operations are the only owner relay UI exception;
      // every workspace command still passes through Rust's LAN allow-list.
      if(command==='get_owner_appearance')return Promise.resolve(currentOwnerAppearance());
      if(command==='set_owner_appearance')return Promise.resolve(applyOwnerAppearance(args));
      // `args` came from the authenticated JSON transfer parser in remote-client.
      const host=pairingHost();
      return host ? host.invoke('owner_relay_invoke',{command,args}) : invokeCommand('owner_relay_invoke',{command,args:args as JsonValue});
    }});if(current!==generation){next.close();return;}session=next;unsubscribe=next.subscribe(value=>connectionChanged(value,current));unsubscribeOwner=next.onOwnerWorkspaceRequest(()=>{if(current===generation)now=Date.now();});status=next.getStatus();}
    catch(reason){if(current===generation){error=String(reason);status='error';}}finally{if(current===generation){creating=false;if(session?.getStatus()==='waiting_for_peer')void createPairingCode();}}
  }
  async function createPairingCode(){
    if(creating||revoking||!session||session.getStatus()!=='waiting_for_peer')return;
    const current=generation,ownedSession=session;creating=true;error='';pairingError='';
    try{
      const invitation=ownedSession.invitation;
      const image=await QRCode.toDataURL(invitation,{width:280,margin:2});
      if(current!==generation||ownedSession.getStatus()!=='waiting_for_peer')return;
      qr=image;
      if(!record)return;
      if(!record.pairingManagementToken){const pending=clone(record);pending.pairingManagementToken=record.pairingRegistration?.revocationToken??bytesToBase64url(randomBytes(32));await persist(pending,current);}
      if(current!==generation||!record)return;
      const saved=record.pairingRegistration;
      const created=await registerPersistentPairingCode(invitation,saved,record.pairingManagementToken);
      if(current!==generation||ownedSession.getStatus()!=='waiting_for_peer'){if(!saved)void revokePairingCode(relay,created).catch(()=>{});return;}
      if(!record){void revokePairingCode(relay,created).catch(()=>{});return;}
      const nextRecord=clone(record);nextRecord.enabled=true;nextRecord.pairingRegistration=created;
      await persist(nextRecord,current);
      if(current===generation)registration=created;
    }catch(reason){if(current===generation&&ownedSession.getStatus()==='waiting_for_peer'){pairingError=reason instanceof Error?reason.message:String(reason);error=pairingError;}}
    finally{if(current===generation)creating=false;}
  }
  function closeRuntime(){generation++;unsubscribe?.();unsubscribe=undefined;unsubscribeOwner?.();unsubscribeOwner=undefined;session?.close();session=null;clearRegistration();qr='';status='off';verificationCode='';creating=false;}
  async function stopListener(){closeRuntime();if(record){const next=clone(record);next.enabled=false;try{await persist(next);}catch{}}}
  async function approve(){if(!session||!record||saving)return;const publicKey=session.getPeerControllerPublicKey(),peer=session.getPeer();if(peer!==null){error='This is a shared visitor connection. Use sharing controls instead of remembering it as a phone.';await session.reject().catch(()=>{});return;}if(!publicKey){error='The phone identity was unavailable. Reject it and create a fresh pairing code.';return;}const current=generation,next=clone(record),timestamp=Date.now(),existing=next.devices.find(device=>device.publicKey===publicKey);if(existing){existing.approvedAt=timestamp;existing.lastAccessAt=timestamp;}else{if(next.devices.length>=32){error='You can remember up to 32 phones. Revoke one before approving another.';return;}next.devices.push({publicKey,name:'Phone '+(next.devices.length+1),approvedAt:timestamp,lastAccessAt:timestamp});}try{await persist(next,current);if(current!==generation||!trusted(publicKey))return;await session?.approve();}catch(reason){if(current===generation)error='Phone was not approved: '+String(reason);}}
  async function reject(){try{await session?.reject();}catch(reason){error=String(reason);}}
  async function approveOwner(){if(!session||!record||saving)return;const publicKey=session.getPeerControllerPublicKey();if(!publicKey){error='The phone identity was unavailable. Reject the owner request and pair again.';return;}const current=generation,next=clone(record),timestamp=Date.now(),existing=next.ownerDevices.find(device=>device.publicKey===publicKey);if(existing){existing.approvedAt=timestamp;existing.lastAccessAt=timestamp;}else{if(next.ownerDevices.length>=32){error='You can grant owner workspace access to up to 32 phones. Revoke one first.';return;}next.ownerDevices.push({publicKey,name:'Phone '+(next.ownerDevices.length+1),approvedAt:timestamp,lastAccessAt:timestamp});}try{await persist(next,current);if(current!==generation||!trustedOwner(publicKey))return;await session.approveOwnerWorkspace();}catch(reason){if(current===generation)error='Owner workspace access was not approved: '+String(reason);}}
  async function rejectOwner(){try{await session?.rejectOwnerWorkspace();}catch(reason){error=String(reason);}}
  async function revokeDevice(publicKey:string){if(!record)return;const current=generation,next=clone(record);next.devices=next.devices.filter(device=>device.publicKey!==publicKey);next.ownerDevices=next.ownerDevices.filter(device=>device.publicKey!==publicKey);record=next;const active=session?.getPeerControllerPublicKey()===publicKey?session:null;const rejection=active?.reject().catch(()=>{});try{await persist(next,current);await rejection;if(current!==generation)return;if(active===session)await active?.reconnectNow();}catch{}}
  async function revokeOwnerDevice(publicKey:string){if(!record)return;const current=generation,next=clone(record);next.ownerDevices=next.ownerDevices.filter(device=>device.publicKey!==publicKey);record=next;const active=session?.getPeerControllerPublicKey()===publicKey?session:null;try{await persist(next,current);if(active===session)await active?.revokeOwnerWorkspace();}catch{}}
  async function revokeAll(){
    if(revoking)return;revoking=true;
    const previousRecord=record,current=++generation,previous=session;
    unsubscribe?.();unsubscribe=undefined;unsubscribeOwner?.();unsubscribeOwner=undefined;
    session=null;registration=null;status='off';qr='';creating=false;verificationCode='';
    // Revoke local authority before any network request. Keep only a disabled,
    // empty-grant record until locator deletion succeeds, so cleanup is retryable.
    const next=previousRecord?clone(previousRecord):null;
    if(next){next.enabled=false;next.devices=[];next.ownerDevices=[];}
    record=next;
    void previous?.reject().catch(()=>{}).finally(()=>previous.close());
    try{
      if(next)await persist(next,current);
      if(current!==generation)return;
      if(next?.pairingManagementToken&&!next.pairingRegistration){
        const invitation=JSON.stringify({version:2,relayUrl:next.relayUrl,room:next.identity.room,publicKey:bytesToBase64url(await exportPairingPublicKey(next.identity.keyPair.publicKey))});
        next.pairingRegistration=await registerPersistentPairingCode(invitation,undefined,next.pairingManagementToken);
      }
      if(next?.pairingRegistration)await revokePairingCode(next.relayUrl,next.pairingRegistration);
      if(current!==generation)return;
      await clearDesktopPairing();if(current===generation)record=null;
    }catch(reason){if(current===generation)error='Saved phone access is stopped. Could not finish removing the PIN; retry Revoke all: '+String(reason);}
    finally{revoking=false;}
  }
  async function savePolicy(){if(!record||saving)return;const trimmed=policyInput.trim(),days=trimmed===''?null:Number(trimmed);if(!(days===null||(Number.isInteger(days)&&days>=1&&days<=MAX_INACTIVITY_DAYS))){error='Enter whole days from 1 to '+MAX_INACTIVITY_DAYS+', or leave this blank for indefinite access.';return;}const current=generation,next=clone(record);next.inactivityDays=days;try{await persist(next,current);if(current!==generation)return;const peer=session?.getPeerControllerPublicKey();if(peer&&status==='connected'&&!trusted(peer)){await session?.reject().catch(()=>{});await session?.reconnectNow();}}catch{}}
  function expiry(device:RememberedController){return deviceExpiresAt(device,record?.inactivityDays??null);}
  function deviceState(device:RememberedController){const expires=expiry(device);if(expires!==null&&expires<=now)return'expired';return session?.getPeerControllerPublicKey()===device.publicKey&&status==='connected'?'connected':'offline';}
  function stamp(value:number){return new Intl.DateTimeFormat(undefined,{dateStyle:'medium',timeStyle:'short'}).format(value);}
  onMount(()=>{
    const stopRecovery=watchReconnectEvents(()=>{
      if(session)void session.reconnectNow().catch(reason=>{error='Could not reconnect remote access: '+String(reason);});
    });
    const current=generation;
    void(async()=>{
      try{
        const saved=await loadDesktopPairing();if(current!==generation||!saved)return;
        record=saved;relay=saved.relayUrl;policyInput=saved.inactivityDays===null?'':String(saved.inactivityDays);
        if(saved.enabled)await startListener(saved);
      }catch(reason){if(current===generation)error='Could not restore remote access: '+String(reason);}
      finally{loaded=true;}
    })();
    return stopRecovery;
  });
  onDestroy(()=>{clearInterval(timer);closeRuntime();});
</script>
{#if open}<div class="remote-control-fallback" class:standalone aria-hidden={standalone?undefined:true}><div class="remote-panel" use:portal={$remoteControlTarget} role="region" aria-label="Remote control">
<header><h2>Remote control</h2></header>
<p>Paired phones can view and control this desktop through an encrypted relay.</p>
<label>Relay address<input bind:value={relay} placeholder="wss://relay.example.com" disabled={!!session||creating||!!record?.pairingRegistration||!!record?.pairingManagementToken}/></label>
<section class="policy"><h3>Remembered phones</h3><label>Inactivity limit (days)<input inputmode="numeric" bind:value={policyInput} placeholder="Indefinite" disabled={!record||saving}/></label><small>Blank means access remains until you revoke it. Each time a phone connects or uses the desktop, its full day limit starts again.</small><button onclick={savePolicy} disabled={!record||saving}>Save policy</button></section>
{#if !session}<button class="primary" disabled={creating||revoking||!loaded} onclick={()=>startListener(record??undefined)}>{creating?'Starting…':record?.enabled?'Restart remote listener':'Enable remote access'}</button>
{:else}
  <p role="status">{statusLabel(status)}</p>
  {#if status==='waiting_for_peer'}
    {#if qr}<img src={qr} alt="Mobile pairing QR code"/>{/if}
    {#if key}
      <div>Fixed pairing PIN<strong class="device-key">{key}</strong></div>
      <small>This PIN stays the same across restarts. Approve a new phone once; remembered phones reconnect automatically.</small>
    {/if}
    {#if !registration}<button class="primary" disabled={creating} onclick={createPairingCode}>{creating?'Creating…':qr?'Retry fixed PIN':'Create fixed PIN'}</button>{/if}
  {/if}
  {#if status==='pending'}
    <p>Check that your phone shows this same verification number:</p>
    <strong class="device-key">{verificationCode}</strong>
    <button class="primary" disabled={saving} onclick={approve}>Numbers match — approve phone</button>
    <button onclick={reject}>Reject</button>
  {/if}
  {#if ownerPending}<section class="owner-request"><h3>Owner workspace requested</h3><p>This phone is asking for the full owner workspace. Its remembered phone approval does not grant this. Only approve if you intend to allow the explicit desktop owner command set.</p><button class="primary" disabled={saving} onclick={approveOwner}>Approve owner workspace</button><button onclick={rejectOwner}>Keep limited phone access</button></section>{/if}
  <button disabled={saving} onclick={stopListener}>Stop listener</button>
{/if}
<section class="devices"><h3>Saved phones</h3>{#each devices as device (device.publicKey)}<article><div><strong>{device.name}</strong><small class:connected={deviceState(device)==='connected'} class:expired={deviceState(device)==='expired'}>{deviceState(device)} · Last access {stamp(device.lastAccessAt)}{#if expiry(device)!==null} · Expires {stamp(expiry(device)!)}{/if}</small></div><button class="danger" aria-label={'Revoke '+device.name} onclick={()=>revokeDevice(device.publicKey)}><ArchiveX size={15}/>Revoke</button></article>{:else}<p class="empty">No phones are remembered yet.</p>{/each}</section>
<section class="devices"><h3>Owner workspace grants</h3><small>Separate explicit grants for the full owner workspace. Removing phone access also removes its owner grant.</small>{#each ownerDevices as device (device.publicKey)}<article><div><strong>{device.name}</strong><small class:connected={session?.getPeerControllerPublicKey()===device.publicKey&&session?.hasOwnerWorkspaceAccess()} class:expired={expiry(device)!==null&&expiry(device)!<=now}>{session?.getPeerControllerPublicKey()===device.publicKey&&session?.hasOwnerWorkspaceAccess()?'active':'inactive'} · Last owner use {stamp(device.lastAccessAt)}{#if expiry(device)!==null} · Expires {stamp(expiry(device)!)}{/if}</small></div><button class="danger" aria-label={'Revoke owner workspace for '+device.name} onclick={()=>revokeOwnerDevice(device.publicKey)}><ArchiveX size={15}/>Revoke</button></article>{:else}<p class="empty">No owner workspace grants.</p>{/each}</section>
{#if devices.length||record}<button class="danger revoke-all" disabled={revoking} onclick={revokeAll}><ShieldCheck size={15}/>Revoke all saved access</button>{/if}
{#if error}<p role="alert">{error}</p>{/if}
<small>Your fixed PIN identifies this desktop; it does not approve a phone. Stopping the listener keeps the PIN and saved phones. Revoke all saved access to remove them.</small>
</div></div>{/if}
<style>
.remote-control-fallback{display:none}.remote-control-fallback.standalone{display:block}.device-key{display:block;font:600 28px var(--mono,monospace);letter-spacing:3px;margin:12px 0;color:var(--accent-ink,var(--accent))}.remote-panel{width:100%;box-sizing:border-box;min-width:0;padding-top:4px;color:var(--ink);font:14px var(--interface-font,'IBM Plex Sans',sans-serif)}.remote-panel header,.devices article{display:flex;align-items:center;justify-content:space-between;gap:12px}.remote-panel h2{font-size:20px;margin:0}.remote-panel h3{font-size:14px;margin:0 0 8px}.remote-panel p{line-height:1.5}.remote-panel label{display:block;margin:12px 0}.remote-panel input{box-sizing:border-box;width:100%;margin-top:6px;border:1px solid var(--line);background:var(--paper);color:inherit;border-radius:8px;padding:10px;font:inherit}.remote-panel button{display:inline-flex;align-items:center;gap:6px;padding:10px;border:0;border-radius:8px;background:var(--soft);color:inherit;cursor:pointer;margin:4px}.remote-panel button:disabled{opacity:.5;cursor:default}.remote-panel button.primary{background:var(--accent);color:var(--on-accent)}.remote-panel img{display:block;width:min(280px,100%);height:auto;margin:12px auto;border-radius:10px}.remote-panel small{display:block;color:var(--muted);line-height:1.5;margin-top:5px}.policy,.devices,.owner-request{border-top:1px solid var(--line);margin-top:16px;padding-top:14px}.devices article{padding:10px 0;border-bottom:1px solid var(--line)}.devices article strong{display:block}.connected{color:var(--accent-ink)!important}.expired{color:#a13a2a!important}:global(:root[data-theme="dark"]) .expired{color:#f0a48e!important}.danger{background:#5a302b!important;color:#fff!important}.revoke-all{margin-top:14px}.empty{color:var(--muted)}
</style>
