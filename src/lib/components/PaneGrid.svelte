<script lang="ts">
  import type { PaneLayout, PaneSplit, PaneTabTransfer } from '$lib/panes';
  type Edge = 'center' | 'left' | 'right' | 'top' | 'bottom';
  let { layout, activePaneId, onactivate, onresize, ondropTab, children }: {
    layout: PaneLayout; activePaneId: string; onactivate: (id: string) => void;
    onresize: (id: string, ratio: number) => void;
    ondropTab: (id: string, edge: Edge, data: PaneTabTransfer) => void;
    children: import('svelte').Snippet<[string]>;
  } = $props();
  let over = $state<{id:string;edge:Edge}|null>(null);
  const mime = 'application/x-monitter-tab';
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
      if(typeof data?.sourcePaneId==='string' && typeof data.id==='string' && ['task','draft','channel'].includes(data.kind)) ondropTab(id,zone,data);
    } catch { /* Ignore unrelated drag data. */ }
  }
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
    <div class="pane-split" class:column={item.axis==='vertical'}>
      <div class="split-child" style={`flex:${item.ratio} 1 0%`}>{@render branch(item.first)}</div>
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (ARIA window splitter is a focusable separator with arrow-key resizing) -->
      <div class="pane-resizer" role="separator" tabindex="0" aria-label="Resize panes" aria-orientation={item.axis==='horizontal'?'vertical':'horizontal'} aria-valuemin="15" aria-valuemax="85" aria-valuenow={Math.round(item.ratio*100)} onpointerdown={event=>resize(event,item)} onkeydown={event=>resizeKey(event,item)}></div>
      <div class="split-child" style={`flex:${1-item.ratio} 1 0%`}>{@render branch(item.second)}</div>
    </div>
  {:else}
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions (contains independently interactive chat controls) -->
    <section class="pane-leaf" data-pane-id={item.id} class:active={activePaneId===item.id} aria-label="Workspace pane" onfocusin={()=>onactivate(item.id)} onpointerdowncapture={()=>onactivate(item.id)} ondragover={event=>dragover(event,item.id)} ondragleave={event=>{if(!(event.relatedTarget instanceof Node) || !(event.currentTarget as HTMLElement).contains(event.relatedTarget))over=null}} ondrop={event=>drop(event,item.id)}>
      {@render children(item.id)}
      {#if over?.id===item.id}<div class="pane-drop" data-edge={over.edge}><span>{over.edge==='center'?'Move tab here':`Split ${over.edge}`}</span></div>{/if}
    </section>
  {/if}
{/snippet}
{@render branch(layout)}
<style>
  .pane-split,.split-child,.pane-leaf{display:flex;flex:1;min-width:0;min-height:0;overflow:hidden}
  .pane-split.column{flex-direction:column}.pane-leaf{position:relative}.pane-leaf.active{outline:1px solid color-mix(in srgb,var(--accent) 25%,transparent);outline-offset:-1px}
  .pane-resizer{position:relative;flex:0 0 5px;cursor:col-resize;background:var(--line);touch-action:none;z-index:2}.column>.pane-resizer{cursor:row-resize}.pane-resizer:hover,.pane-resizer:focus-visible{background:var(--accent);outline:0}
  .pane-drop{position:absolute;inset:6px;z-index:30;display:grid;place-items:center;border:2px solid var(--accent);border-radius:10px;background:color-mix(in srgb,var(--accent) 16%,var(--panel));opacity:.94;pointer-events:none}
  .pane-drop[data-edge=left]{right:50%}.pane-drop[data-edge=right]{left:50%}.pane-drop[data-edge=top]{bottom:50%}.pane-drop[data-edge=bottom]{top:50%}.pane-drop span{padding:8px;border-radius:6px;background:var(--panel);color:var(--ink);font-size:12px}
</style>
