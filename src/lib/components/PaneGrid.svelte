<script lang="ts">
  import { paneIds, type PaneLayout, type PaneSplit, type PaneTabTransfer } from '$lib/panes';
  type Edge = 'center' | 'left' | 'right' | 'top' | 'bottom';
  let { layout, activePaneId, expandedPaneId=null, dimInactivePanes=true, inactivePaneOpacity=.6, focusFollowsMouse=false, pointerDrag=null, onPointerDragEnd, onactivate, onresize, ondropTab, children }: {
    layout: PaneLayout; activePaneId: string; expandedPaneId?:string|null; dimInactivePanes?:boolean;inactivePaneOpacity?:number; focusFollowsMouse?:boolean; onactivate: (id: string) => void;
    onresize: (id: string, ratio: number) => void;
    pointerDrag?: {tab:PaneTabTransfer;pointerId:number;startX:number;startY:number}|null; onPointerDragEnd?:()=>void; ondropTab: (id: string, edge: Edge, data: PaneTabTransfer, before?: {kind:PaneTabTransfer['kind'];id:string}) => void;
    children: import('svelte').Snippet<[string]>;
  } = $props();
  let over = $state<{id:string;edge:Edge}|null>(null);
  const mime = 'application/x-monitter-tab';
  function hoverPane(event:PointerEvent,id:string) {
    if(!focusFollowsMouse || activePaneId===id || event.pointerType!=='mouse' || event.buttons || !document.hasFocus())return;
    // Menus and dialogs keep keyboard focus until dismissed. Dragging must not
    // redirect input or disrupt selections while crossing a pane boundary.
    if(document.querySelector('dialog[open], [role="dialog"], :popover-open'))return;
    onactivate(id);
    const pane=event.currentTarget as HTMLElement;
    const input=pane.querySelector<HTMLElement>('.terminal-pane .xterm-helper-textarea')
      ?? pane.querySelector<HTMLElement>('textarea[aria-label="Task message"], textarea[aria-label="Channel message"]')
      ?? pane.querySelector<HTMLElement>('.messages');
    (input??pane).focus({preventScroll:true});
  }
  function edge(event: DragEvent, box: DOMRect): Edge {
    const x=(event.clientX-box.left)/box.width, y=(event.clientY-box.top)/box.height;
    const closest=Math.min(x,1-x,y,1-y);
    if(closest>.24) return 'center';
    return closest===x?'left':closest===1-x?'right':closest===y?'top':'bottom';
  }
  function dragover(event:DragEvent,id:string) {
    if(!event.dataTransfer?.types.includes(mime)) return;
    event.preventDefault(); event.stopPropagation(); event.dataTransfer.dropEffect='move';
    over={id,edge:edge(event,(event.currentTarget as HTMLElement).getBoundingClientRect())};
  }
  function drop(event:DragEvent,id:string) {
    const zone=over?.id===id?over.edge:'center'; over=null;
    if(!event.dataTransfer?.types.includes(mime)) return;
    event.preventDefault();event.stopPropagation();
    try {
      const data=JSON.parse(event.dataTransfer.getData(mime));
      if(typeof data?.sourcePaneId==='string' && typeof data.id==='string' && ['task','draft','channel','terminal','settings'].includes(data.kind)) ondropTab(id,zone,data);
    } catch { /* Ignore unrelated drag data. */ }
  }
  function pointerTarget(event: PointerEvent) {
    const target=document.elementFromPoint(event.clientX,event.clientY);
    const pane=target?.closest<HTMLElement>('.pane-leaf[data-pane-id]');
    if(!pane || !pane.closest('.app-shell')) return null;
    const box=pane.getBoundingClientRect(), tabBar=target?.closest<HTMLElement>('.tabs');
    if(tabBar) {
      const entries=Array.from(tabBar.querySelectorAll<HTMLElement>('[data-tab-kind][data-tab-id]'));
      const marker=target?.closest<HTMLElement>('[data-tab-kind][data-tab-id]') ?? null;
      const after=marker ? event.clientX>marker.getBoundingClientRect().left+marker.getBoundingClientRect().width/2 : true;
      const next=marker ? entries[entries.indexOf(marker)+(after?1:0)] : event.clientX<(entries[0]?.getBoundingClientRect().left??0)?entries[0]:undefined;
      const before=next?{kind:next.dataset.tabKind as PaneTabTransfer['kind'],id:next.dataset.tabId!}:undefined;
      return {id:pane.dataset.paneId!,edge:'center' as const,before,bar:tabBar,marker:marker??next??entries.at(-1)??tabBar,after:marker?after:!next};
    }
    const x=(event.clientX-box.left)/box.width, y=(event.clientY-box.top)/box.height;
    const closest=Math.min(x,1-x,y,1-y);
    const zone:Edge=closest>.24?'center':closest===x?'left':closest===1-x?'right':closest===y?'top':'bottom';
    return {id:pane.dataset.paneId!,edge:zone,before:undefined,bar:null,marker:null,after:false};
  }
  $effect(() => {
    const drag=pointerDrag;
    if(!drag) return;
    let moved=false,marker:HTMLElement|null=null;
    const source=document.querySelector<HTMLElement>(`.pane-leaf[data-pane-id="${CSS.escape(drag.tab.sourcePaneId)}"] [data-tab-kind="${drag.tab.kind}"][data-tab-id="${CSS.escape(drag.tab.id)}"] .tab`);
    const cursor=document.body.style.cursor;
    const clearMarker=()=>{marker?.removeAttribute('data-tab-insert');marker=null;};
    const clear=()=>{clearMarker();over=null;document.body.style.cursor=cursor;source?.removeAttribute('data-tab-dragging');if(source?.hasPointerCapture(drag.pointerId))source.releasePointerCapture(drag.pointerId);};
    const cancel=()=>{clear();onPointerDragEnd?.();};
    const move=(event:PointerEvent)=>{
      if(event.pointerId!==drag.pointerId)return;
      if(!moved && Math.hypot(event.clientX-drag.startX,event.clientY-drag.startY)<6)return;
      if(!moved){moved=true;source?.setPointerCapture(drag.pointerId);source?.setAttribute('data-tab-dragging','true');document.body.style.cursor='grabbing';}
      event.preventDefault();clearMarker();
      const target=pointerTarget(event);
      over=target && !target.bar?{id:target.id,edge:target.edge}:null;
      if(target?.bar){marker=target.marker;marker?.setAttribute('data-tab-insert',target.after?'after':'before');const box=target.bar.getBoundingClientRect();if(event.clientX<box.left+24)target.bar.scrollLeft-=12;else if(event.clientX>box.right-24)target.bar.scrollLeft+=12;}
    };
    const finish=(event:PointerEvent)=>{
      if(event.pointerId!==drag.pointerId)return;
      if(moved){
        const suppress=(click:MouseEvent)=>{click.preventDefault();click.stopImmediatePropagation();};
        window.addEventListener('click',suppress,true);setTimeout(()=>window.removeEventListener('click',suppress,true),0);
        const target=pointerTarget(event);if(target)ondropTab(target.id,target.edge,drag.tab,target.before);
      }
      cancel();
    };
    const key=(event:KeyboardEvent)=>{if(event.key==='Escape'){event.preventDefault();cancel();}};
    window.addEventListener('pointermove',move,{passive:false});window.addEventListener('pointerup',finish);window.addEventListener('pointercancel',cancel);window.addEventListener('blur',cancel);window.addEventListener('keydown',key);
    return ()=>{clear();window.removeEventListener('pointermove',move);window.removeEventListener('pointerup',finish);window.removeEventListener('pointercancel',cancel);window.removeEventListener('blur',cancel);window.removeEventListener('keydown',key);};
  });
  function resize(event:PointerEvent,node:PaneSplit) {
    const handle=event.currentTarget as HTMLElement, container=handle.parentElement!;
    const rect=container.getBoundingClientRect(), horizontal=node.axis==='horizontal';
    const size=horizontal?rect.width:rect.height, origin=horizontal?rect.left:rect.top;
    handle.setPointerCapture(event.pointerId);event.preventDefault();
    const move=(next:PointerEvent)=>onresize(node.id,Math.max(.15,Math.min(.85,((horizontal?next.clientX:next.clientY)-origin)/size)));
    const finish=()=>{handle.removeEventListener('pointermove',move);handle.removeEventListener('pointerup',finish);handle.removeEventListener('pointercancel',finish);handle.removeEventListener('lostpointercapture',finish);};
    handle.addEventListener('pointermove',move);handle.addEventListener('pointerup',finish);handle.addEventListener('pointercancel',finish);handle.addEventListener('lostpointercapture',finish);
  }
  function resizeKey(event:KeyboardEvent,node:PaneSplit) {
    const decrease=node.axis==='horizontal'?'ArrowLeft':'ArrowUp', increase=node.axis==='horizontal'?'ArrowRight':'ArrowDown';
    if(event.key===decrease || event.key===increase) {event.preventDefault();onresize(node.id,Math.max(.15,Math.min(.85,node.ratio+(event.key===increase?.05:-.05))));}
  }
</script>
<svelte:window ondragend={()=>over=null}/>
{#snippet branch(item:PaneLayout)}
  {#if 'axis' in item}
    <div class="pane-split" data-split-id={item.id} class:column={item.axis==='vertical'}>
      <div class="split-child" class:focus-hidden={!!expandedPaneId && !paneIds(item.first).includes(expandedPaneId)} style={`flex:${expandedPaneId?1:item.ratio} 1 0%`}>{@render branch(item.first)}</div>
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (ARIA window splitter is a focusable separator with arrow-key resizing) -->
      <div class="pane-resizer" class:focus-hidden={!!expandedPaneId} role="separator" tabindex="0" aria-label="Resize panes" aria-orientation={item.axis==='horizontal'?'vertical':'horizontal'} aria-valuemin="15" aria-valuemax="85" aria-valuenow={Math.round(item.ratio*100)} onpointerdown={event=>resize(event,item)} onkeydown={event=>resizeKey(event,item)}></div>
      <div class="split-child" class:focus-hidden={!!expandedPaneId && !paneIds(item.second).includes(expandedPaneId)} style={`flex:${expandedPaneId?1:1-item.ratio} 1 0%`}>{@render branch(item.second)}</div>
    </div>
  {:else}
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions (contains independently interactive chat controls) -->
    <section class="pane-leaf" data-pane-id={item.id} data-focus-follows-mouse={focusFollowsMouse} class:active={activePaneId===item.id} class:dimmed={dimInactivePanes && activePaneId!==item.id} style:opacity={activePaneId===item.id || !dimInactivePanes ? 1 : Math.max(.1,Math.min(.9,inactivePaneOpacity))} aria-label="Workspace pane" tabindex="-1" onpointerenter={event=>hoverPane(event,item.id)} onfocusin={()=>onactivate(item.id)} onpointerdowncapture={()=>onactivate(item.id)} ondragover={event=>dragover(event,item.id)} ondragleave={event=>{if(!(event.relatedTarget instanceof Node) || !(event.currentTarget as HTMLElement).contains(event.relatedTarget))over=null}} ondrop={event=>drop(event,item.id)}>
      {@render children(item.id)}
      {#if over?.id===item.id}<div class="pane-drop" data-edge={over.edge}><span>{over.edge==='center'?'Move tab here':`Split ${over.edge}`}</span></div>{/if}
    </section>
  {/if}
{/snippet}
{@render branch(layout)}
<style>
  :global([data-tab-insert="before"]){box-shadow:inset 2px 0 var(--accent)!important}
  :global([data-tab-insert="after"]){box-shadow:inset -2px 0 var(--accent)!important}
  :global([data-tab-dragging="true"]){opacity:.5}
  :global(.tabs .tab){-webkit-user-drag:none;user-select:none;touch-action:none}

  .pane-split,.split-child,.pane-leaf{display:flex;flex:1;min-width:0;min-height:0;overflow:hidden}
  .focus-hidden{display:none!important}
  .pane-split.column{flex-direction:column}.pane-leaf{position:relative;transition:opacity .14s ease,filter .14s ease}.pane-leaf.active{outline:none}.pane-leaf.dimmed{filter:grayscale(1)}
  .pane-resizer{position:relative;flex:0 0 1px;cursor:col-resize;background:var(--line);touch-action:none;z-index:2}.pane-resizer::after{content:"";position:absolute;inset:0 -4px}.column>.pane-resizer{cursor:row-resize}.column>.pane-resizer::after{inset:-4px 0}.pane-resizer:hover,.pane-resizer:focus-visible{background:var(--accent);outline:0}
  .pane-drop{position:absolute;inset:6px;z-index:30;display:grid;place-items:center;border:2px solid var(--accent);border-radius:10px;background:color-mix(in srgb,var(--accent) 16%,var(--panel));opacity:.94;pointer-events:none}
  .pane-drop[data-edge=left]{right:50%}.pane-drop[data-edge=right]{left:50%}.pane-drop[data-edge=top]{bottom:50%}.pane-drop[data-edge=bottom]{top:50%}.pane-drop span{padding:8px;border-radius:6px;background:var(--panel);color:var(--ink);font-size:calc(12px * var(--interface-font-ratio, 1))}
  @media(prefers-reduced-motion:reduce){.pane-leaf{transition:none}}
</style>
