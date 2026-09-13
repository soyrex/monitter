<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import { animateMotion, motionEnabled } from '$lib/motion';
  import { paneIds, type PaneLayout, type PaneSplit, type PaneTabTransfer } from '$lib/panes';
  type Edge = 'center' | 'left' | 'right' | 'top' | 'bottom';
  let { layout, activePaneId, expandedPaneId=null, dimInactivePanes=true, inactivePaneOpacity=.6, focusFollowsMouse=false, pointerDrag=null, onPointerDragEnd, onactivate, onresize, ondropTab, children }: {
    layout: PaneLayout; activePaneId: string; expandedPaneId?:string|null; dimInactivePanes?:boolean;inactivePaneOpacity?:number; focusFollowsMouse?:boolean; onactivate: (id: string) => void;
    onresize: (id: string, ratio: number) => void;
    pointerDrag?: {tab:PaneTabTransfer;pointerId:number;startX:number;startY:number}|null; onPointerDragEnd?:()=>void; ondropTab: (id: string, edge: Edge, data: PaneTabTransfer, before?: {kind:PaneTabTransfer['kind'];id:string}) => void;
    children: import('svelte').Snippet<[string]>;
  } = $props();
  let over = $state<{id:string;edge:Edge}|null>(null);
  let pointerResizing = false;
  let geometryReady = false;
  let gridRoot: HTMLElement;
  let geometryGeneration = 0;
  const geometryAnimations = new Set<Animation>();
  const mime = 'application/x-monitter-tab';

  function layoutSignature(item: PaneLayout): string {
    return 'axis' in item
      ? `${item.id}:${item.axis}:${item.ratio}:${layoutSignature(item.first)}:${layoutSignature(item.second)}`
      : item.id;
  }
  function cancelGeometryAnimations() {
    for (const animation of geometryAnimations) animation.cancel();
    geometryAnimations.clear();
  }
  function addGeometryAnimation(animation: Animation | null) {
    if (!animation) return;
    geometryAnimations.add(animation);
    void animation.finished.then(
      () => geometryAnimations.delete(animation),
      () => geometryAnimations.delete(animation),
    );
  }
  onDestroy(cancelGeometryAnimations);

  // FLIP only retained panes after discrete layout operations. Pointer resizing
  // updates flex geometry directly; animating those intermediate positions makes
  // terminal refits and splitter feedback feel delayed.
  $effect.pre(() => {
    layoutSignature(layout);
    expandedPaneId;
    pointerDrag;
    const generation = ++geometryGeneration;
    cancelGeometryAnimations();
    if (!geometryReady) { geometryReady = true; return; }
    if (pointerResizing || pointerDrag || !motionEnabled() || !gridRoot) return;
    const before = new Map(Array.from(gridRoot.querySelectorAll<HTMLElement>('.pane-leaf[data-pane-id]')).map(node => [node.dataset.paneId!, node.getBoundingClientRect()]));
    void tick().then(() => {
      if (generation !== geometryGeneration || pointerResizing || pointerDrag || !motionEnabled()) return;
      for (const node of gridRoot?.querySelectorAll<HTMLElement>('.pane-leaf[data-pane-id]') ?? []) {
        const previous = before.get(node.dataset.paneId!);
        if (!previous) {
          // A new pane may contain a just-restored terminal; opacity-only entry
          // avoids changing its final geometry or delaying attachment/focus.
          addGeometryAnimation(animateMotion(node, [{ opacity: 0.96 }, { opacity: 1 }], { duration: 180, easing: 'ease-out' }));
          continue;
        }
        const next = node.getBoundingClientRect();
        const x = previous.left - next.left;
        const y = previous.top - next.top;
        if (Math.abs(x) < 1 && Math.abs(y) < 1) continue;
        addGeometryAnimation(animateMotion(node, [
          { transform: `translate(${x}px, ${y}px)`, opacity: 0.96 },
          { transform: 'translate(0, 0)', opacity: 1 },
        ], { duration: 200, easing: 'ease-out' }));
      }
    });
  });
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
    pointerResizing=true;handle.setPointerCapture(event.pointerId);event.preventDefault();
    const move=(next:PointerEvent)=>onresize(node.id,Math.max(.15,Math.min(.85,((horizontal?next.clientX:next.clientY)-origin)/size)));
    const finish=()=>{pointerResizing=false;handle.removeEventListener('pointermove',move);handle.removeEventListener('pointerup',finish);handle.removeEventListener('pointercancel',finish);handle.removeEventListener('lostpointercapture',finish);};
    handle.addEventListener('pointermove',move);handle.addEventListener('pointerup',finish);handle.addEventListener('pointercancel',finish);handle.addEventListener('lostpointercapture',finish);
  }
  function resizeKey(event:KeyboardEvent,node:PaneSplit) {
    const decrease=node.axis==='horizontal'?'ArrowLeft':'ArrowUp', increase=node.axis==='horizontal'?'ArrowRight':'ArrowDown';
    if(event.key===decrease || event.key===increase) {event.preventDefault();onresize(node.id,Math.max(.15,Math.min(.85,node.ratio+(event.key===increase?.05:-.05))));}
  }
</script>
<svelte:window ondragend={()=>over=null}/>
{#snippet branch(node:PaneLayout)}
  <!-- Freeze each branch identity while a split is pruned or promoted. Outgoing
       children must not read child properties from an already-replaced parent. -->
  {#each [node] as item (item.id)}
  {#if 'axis' in item}
    <div class="pane-split" data-split-id={item.id} class:column={item.axis==='vertical'}>
      <div class="split-child" class:focus-hidden={!!expandedPaneId && !paneIds(item.first).includes(expandedPaneId)} style={`flex:${expandedPaneId?1:item.ratio} 1 0%`}>{@render branch(item.first)}</div>
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (ARIA window splitter is a focusable separator with arrow-key resizing) -->
      <div class="pane-resizer" class:focus-hidden={!!expandedPaneId} role="separator" tabindex="0" aria-label="Resize panes" aria-orientation={item.axis==='horizontal'?'vertical':'horizontal'} aria-valuemin="15" aria-valuemax="85" aria-valuenow={Math.round(item.ratio*100)} onpointerdown={event=>resize(event,item)} onkeydown={event=>resizeKey(event,item)}></div>
      <div class="split-child" class:focus-hidden={!!expandedPaneId && !paneIds(item.second).includes(expandedPaneId)} style={`flex:${expandedPaneId?1:1-item.ratio} 1 0%`}>{@render branch(item.second)}</div>
    </div>
  {:else}
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions (contains independently interactive chat controls) -->
    <section class="pane-leaf" data-pane-id={item.id} data-focus-follows-mouse={focusFollowsMouse} class:active={activePaneId===item.id} class:dimmed={dimInactivePanes && activePaneId!==item.id} style:--pane-dim-strength={activePaneId===item.id || !dimInactivePanes ? 0 : 1-Math.max(.1,Math.min(.9,inactivePaneOpacity))} style:--pane-dim-visible={dimInactivePanes && activePaneId!==item.id ? 1 : 0} aria-label="Workspace pane" tabindex="-1" onpointerenter={event=>hoverPane(event,item.id)} onfocusin={()=>onactivate(item.id)} onpointerdowncapture={()=>onactivate(item.id)} ondragover={event=>dragover(event,item.id)} ondragleave={event=>{if(!(event.relatedTarget instanceof Node) || !(event.currentTarget as HTMLElement).contains(event.relatedTarget))over=null}} ondrop={event=>drop(event,item.id)}>
      {@render children(item.id)}
      <div class="pane-dim-overlay" aria-hidden="true"></div>
      {#if over?.id===item.id}<div class="pane-drop" data-edge={over.edge}><span>{over.edge==='center'?'Move tab here':`Split ${over.edge}`}</span></div>{/if}
    </section>
  {/if}
  {/each}
{/snippet}
<div class="pane-grid-root" bind:this={gridRoot}>{@render branch(layout)}</div>
<style>
  :global([data-tab-insert="before"]){box-shadow:inset 2px 0 var(--accent)!important}
  :global([data-tab-insert="after"]){box-shadow:inset -2px 0 var(--accent)!important}
  :global([data-tab-dragging="true"]){opacity:.5}
  :global(.tabs .tab){-webkit-user-drag:none;user-select:none;touch-action:none}

  .pane-grid-root,.pane-split,.split-child,.pane-leaf{display:flex;flex:1;min-width:0;min-height:0;overflow:hidden}
  .focus-hidden{display:none!important}
  .pane-split.column{flex-direction:column}.pane-leaf{position:relative;background:var(--paper)}.pane-leaf.active{outline:none}
  .pane-dim-overlay{position:absolute;inset:0;z-index:20;pointer-events:none;opacity:var(--pane-dim-visible);background:color-mix(in srgb,#f4f4f4 calc(var(--pane-dim-strength) * 100%),transparent);transition:opacity .14s ease}
  :global(:root[data-theme="dark"]) .pane-dim-overlay{background:color-mix(in srgb,#000 calc(var(--pane-dim-strength) * 100%),transparent)}
  @media(prefers-color-scheme:dark){:global(:root[data-theme="system"]) .pane-dim-overlay{background:color-mix(in srgb,#000 calc(var(--pane-dim-strength) * 100%),transparent)}}
  .pane-resizer{position:relative;flex:0 0 1px;cursor:col-resize;background:var(--line);touch-action:none;z-index:2}.pane-resizer::after{content:"";position:absolute;inset:0 -4px}.column>.pane-resizer{cursor:row-resize}.column>.pane-resizer::after{inset:-4px 0}.pane-resizer:hover,.pane-resizer:focus-visible{background:var(--accent);outline:0}
  .pane-drop{position:absolute;inset:6px;z-index:30;display:grid;place-items:center;border:2px solid var(--accent);border-radius:10px;background:color-mix(in srgb,var(--accent) 16%,var(--panel));opacity:.94;pointer-events:none}
  .pane-drop[data-edge=left]{right:50%}.pane-drop[data-edge=right]{left:50%}.pane-drop[data-edge=top]{bottom:50%}.pane-drop[data-edge=bottom]{top:50%}.pane-drop span{padding:8px;border-radius:6px;background:var(--panel);color:var(--ink);font-size:calc(12px * var(--interface-font-ratio, 1))}
  @media(prefers-reduced-motion:reduce){.pane-dim-overlay{transition:none}}
</style>
