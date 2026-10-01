<script>
  import { onMount } from 'svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { guardBrowserResize } from '../../src/lib/browser-resize';
  let resizing = $state(false);
  const native = isTauri();
  const nativeUrl = typeof __BROWSER_SMOKE_URL__ === 'undefined' ? 'https://example.test' : __BROWSER_SMOKE_URL__;
  const nativeTab = '299220af-cadd-48e0-a4da-a4f61a2f7a0a';
  let nativeStatus = $state('Native fixture starting');
  let nativeReady = false, nativeTail = Promise.resolve(), nativeRevision = 0, lastRect;
  onMount(() => {
    const cleanup = guardBrowserResize(value => { resizing = value; });
    if (native) void (async () => {
      await getCurrentWebview().setZoom(1.25);
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      const rect = document.querySelector('.browser-native-viewport').getBoundingClientRect();
      await invoke('browser_open', { tabId:nativeTab, url:'about:blank', bounds:bounds(rect) });
      await invoke('browser_navigate', { tabId:nativeTab, url:nativeUrl });
      nativeReady = true; nativeStatus = 'Native fixture ready';
      if(lastRect) report(lastRect);
    })().catch(reason => { nativeStatus = String(reason); });
    return cleanup;
  });
  function bounds(rect) { return {x:Math.round(rect.x*1.25),y:Math.round(rect.y*1.25),width:Math.round(rect.width*1.25),height:Math.round(rect.height*1.25)}; }
  function report(rect) {
    window.paneQA.reports.push(rect);lastRect=rect;
    if(!native || !nativeReady)return;
    const revision=++nativeRevision;
    nativeTail=nativeTail.catch(()=>{}).then(async()=>{
      if(revision!==nativeRevision)return;
      await invoke('browser_set_layout',{tabId:nativeTab,bounds:bounds(rect),visible:rect.visible});
      nativeStatus=rect.visible?'Native fixture visible':'Native fixture hidden';
    }).catch(reason=>{nativeStatus=String(reason);});
  }
  function holdResize() { document.querySelector('.sidebar-resize').dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,pointerId:59,button:0})); }
  function releaseResize() { window.dispatchEvent(new PointerEvent('pointerup',{pointerId:59})); }

  import PaneGrid from '../../src/lib/components/PaneGrid.svelte';
  import BrowserTabPanel from '../../src/lib/components/BrowserTabPanel.svelte';
  let layout=$state({id:'root',axis:'horizontal',ratio:.4,first:{id:'other'},second:{id:'browser'}});
  let revision=$derived(JSON.stringify(layout));
  window.paneQA={
    resize:ratio=>layout={...layout,ratio},
    closeOther:()=>layout={id:'browser'},
    move:()=>layout={id:'root2',axis:'vertical',ratio:.3,first:{id:'other'},second:{id:'browser'}},
    reports:[],
  };
</script>
{#if native}<div class="native-controls"><strong>Isolated native browser resize test</strong><span role="status">{nativeStatus}</span><button onclick={holdResize}>Hold resize placeholder</button><button onclick={()=>layout={...layout,ratio:layout.ratio>.5?.35:.7}}>Resize hidden pane</button><button onclick={releaseResize}>Finish resize</button></div>{/if}
<div class="host">
<PaneGrid {layout} activePaneId="browser" onactivate={()=>{}} onresize={(id,ratio)=>layout={...layout,ratio}} ondropTab={()=>{}}>
  {#snippet children(id)}
    {#if id==='browser'}<div class="fill"><BrowserTabPanel tab={{id:'test',url:nativeUrl,title:'Test',unloaded:false}} active={!resizing} {resizing} layoutRevision={revision} onLayout={report}/></div>{:else}<div>Other pane<div class="sidebar-resize" role="separator" aria-label="Resize fixture sidebar" ></div></div>{/if}
  {/snippet}
</PaneGrid>
</div>
<style>.native-controls{display:flex;align-items:center;gap:12px;height:76px;padding:0 12px;font:13px sans-serif}.native-controls button{padding:10px}.host{display:flex;width:900px;height:620px}.sidebar-resize{width:12px;height:120px;background:#888;cursor:col-resize}.fill{flex:1;min-width:0;min-height:0}</style>
