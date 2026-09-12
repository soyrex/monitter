<script lang="ts">
 import { onDestroy } from 'svelte';
 import { X } from '@lucide/svelte';
 import QRCode from 'qrcode';
 import { getBridge } from '$lib/bridge';
 import { createDesktopSession } from '$lib/controller/remote-client';
 import { DEFAULT_RELAY, registerPairingCode, revokePairingCode, type PairingRegistration } from '$lib/controller/pairing-code';
 let { open = $bindable(false) }: { open?: boolean } = $props();
 let relay=$state(DEFAULT_RELAY), qr=$state(''), status=$state('off'), error=$state('');
 let verificationCode=$state(''), creating=$state(false), now=$state(Date.now());
 let registration=$state<PairingRegistration|null>(null);
 let session=$state<Awaited<ReturnType<typeof createDesktopSession>>|null>(null);
 let unsubscribe:(()=>void)|undefined;
 let generation=0;
 const timer=setInterval(()=>now=Date.now(),1000);
 const key=$derived(registration && registration.expiresAt>now ? registration.code.match(/.{3}/g)?.join(' ') : '');
 function clearRegistration(){const old=registration;registration=null;if(old)void revokePairingCode(relay,old).catch(()=>{});}
 async function start(){
   stop();const current=++generation;creating=true;error='';
   try {
     if(!getBridge().available)throw new Error('Open Remote control inside the Monitter desktop app.');
     const bridge = getBridge();
     // The bridge returns either a legacy Snapshot or a durable fast-send
     // receipt. The dispatcher only exposes the receipt after the paired phone
     // offers that capability, so an installed older phone still sees a Snapshot.
     const controllerBridge = { ...bridge, sendMessage: (taskId: string, text: string) => bridge.sendMessage(taskId, text) };
     const next=await createDesktopSession(relay,controllerBridge);
     if(current!==generation){next.close();return;} session=next;
     unsubscribe=next.subscribe(value=>{status=value.status;verificationCode=value.verificationCode??'';if(value.error)error=value.error;if(['pending','connected','closed','error','rejected'].includes(status))clearRegistration();});
     qr=await QRCode.toDataURL(next.invitation,{width:280,margin:2});
     const created=await registerPairingCode(next.invitation);
     if(current!==generation||!['connecting','waiting_for_peer'].includes(next.getStatus())){void revokePairingCode(relay,created).catch(()=>{});return;}
     registration=created;
   } catch(e){if(current===generation)error=String(e);}
   finally {if(current===generation)creating=false;}
 }
 function stop(){generation++;unsubscribe?.();unsubscribe=undefined;session?.close();session=null;clearRegistration();qr='';status='off';verificationCode='';creating=false;}
 async function approve(){try{await session?.approve();}catch(e){error=String(e);}}
 async function reject(){try{await session?.reject();}catch(e){error=String(e);}}
 onDestroy(()=>{clearInterval(timer);stop();});
</script>
{#if open}<div class="remote-panel" role="dialog" aria-label="Remote control">
<header><h2>Remote control</h2><button aria-label="Close remote settings" onclick={()=>open=false}><X size={20}/></button></header>
<p>Scan this desktop from your phone, or enter its nine-digit device key.</p>
<label>Relay address<input bind:value={relay} placeholder="wss://relay.example.com" disabled={!!session||creating}/></label>
{#if !session}<button class="primary" disabled={creating} onclick={start}>{creating?'Creating…':'Create pairing code'}</button>
{:else}
<p role="status">{status.replaceAll('_',' ')}</p>
{#if qr && ['connecting','waiting_for_peer'].includes(status)}<img src={qr} alt="Mobile pairing QR code"/>{/if}
{#if key}<div>One-time device key<strong class="device-key">{key}</strong></div><small>Expires in {Math.max(0,Math.ceil(((registration?.expiresAt??0)-now)/1000))} seconds. One use only.</small>{/if}
{#if status==='pending'}<p>Check that your phone shows this same verification number:</p><strong class="device-key">{verificationCode}</strong><button class="primary" onclick={approve}>Numbers match — approve phone</button><button onclick={reject}>Reject</button>{/if}
{#if ['closed','error','rejected'].includes(status) || (registration && !key)}<button class="primary" onclick={start}>Create a fresh pairing code</button>{/if}
<button onclick={stop}>Disconnect and revoke session</button>
{/if}
{#if error}<p role="alert">{error}</p>{/if}
<small>Keep this desktop open. The relay forwards encrypted messages; work runs on this desktop.</small>
</div>{/if}
<style>
.device-key{display:block;font:600 28px var(--mono,monospace);letter-spacing:3px;margin:12px 0;color:var(--accent-ink,var(--accent))}
.remote-panel{position:fixed;right:18px;bottom:18px;z-index:100;width:min(360px,calc(100vw - 36px));max-height:85vh;overflow:auto;padding:22px;border:1px solid var(--line);border-radius:18px;background:var(--panel);color:var(--ink);font:14px var(--interface-font,'IBM Plex Sans',sans-serif);box-shadow:0 12px 40px #0008}.remote-panel header{display:flex;align-items:center;justify-content:space-between}.remote-panel h2{font-size:20px;margin:0}.remote-panel p{line-height:1.5}.remote-panel label{display:block;margin:14px 0}.remote-panel input{box-sizing:border-box;width:100%;margin-top:6px;border:1px solid var(--line);background:var(--paper);color:inherit;border-radius:8px;padding:10px;font:inherit}.remote-panel button{padding:10px;border:0;border-radius:8px;background:var(--soft);color:inherit;cursor:pointer;margin:4px}.remote-panel button.primary{background:var(--accent);color:var(--on-accent)}.remote-panel img{display:block;max-width:100%;margin:12px auto;border-radius:10px}.remote-panel small{display:block;color:var(--muted);line-height:1.5;margin-top:14px}
</style>
