<script lang="ts">
  import { onMount } from 'svelte';
  let { side, collapsed=false, oncollapse }: {side:'left'|'right';collapsed?:boolean;oncollapse?:(value:boolean)=>void}=$props();
  let expandedWidth=0;
  let handle:HTMLDivElement;
  let activeCleanup=()=>{};
  let width=$state(0),maximum=$state(0);
  const minimum=$derived(side==='left'?230:260);
  const variable=$derived(`--${side}-sidebar-width`);
  const key=$derived(`monitter.${side}-sidebar-width.v1`);
  function bounds(){
    const parent=handle?.parentElement;
    const area=side==='right'?parent?.closest('.task-layout'):null;
    const overlay=area?.classList.contains('compact-detail')??false;
    const remaining=area?area.getBoundingClientRect().width-(overlay?24:300):Infinity;
    return Math.max(0,Math.min(window.innerWidth*(overlay?1:.4),remaining));
  }
  function apply(value:number){maximum=bounds();expandedWidth=Math.min(maximum,Math.max(minimum,value));width=expandedWidth;document.documentElement.style.setProperty(variable,`${expandedWidth}px`);}
  function save(){try{localStorage.setItem(key,String(expandedWidth));}catch{/* The current width still applies when storage is unavailable. */}}
  function fitContent(){
    if(side!=='left')return;
    if(collapsed){oncollapse?.(false);requestAnimationFrame(fitContent);return;}
    const parent=handle.parentElement;if(!parent)return;
    const parentRect=parent.getBoundingClientRect(),scale=parentRect.width/parent.offsetWidth||1;
    const canvas=document.createElement('canvas'),context=canvas.getContext('2d');
    if(!context)return;
    let desired=minimum;
    const selectors='.agent-name b,.agent-name small,.chat-copy > span,.project-name > span:first-of-type,.channel-row > span,.section-label > span,.empty-tree,.workspace-tab-row .task-select > span:last-child';
    for(const element of parent.querySelectorAll<HTMLElement>(selectors)){
      if(!element.offsetParent||!element.textContent?.trim())continue;
      const style=getComputedStyle(element),rect=element.getBoundingClientRect();context.font=style.font;
      const left=(rect.left-parentRect.left)/scale;
      desired=Math.max(desired,left+context.measureText(element.textContent.trim()).width+48);
    }
    apply(Math.ceil(desired));save();
  }
  function drag(event:PointerEvent){
    if(event.button!==0)return;
    activeCleanup();event.preventDefault();event.stopPropagation();
    const start=handle.parentElement!.getBoundingClientRect().width,x=event.clientX,id=event.pointerId;
    const startedCollapsed=side==='left'&&collapsed;
    const savedWidth=expandedWidth || start;
    let isCollapsed=startedCollapsed;
    const setCollapsed=(value:boolean)=>{isCollapsed=value;oncollapse?.(value);};
    const restore=()=>{if(side==='left')setCollapsed(startedCollapsed);apply(savedWidth);};
    const cursor=document.body.style.cursor;document.body.style.cursor='col-resize';handle.setPointerCapture(id);
    const move=(next:PointerEvent)=>{
      if(next.pointerId!==id)return;
      const delta=(next.clientX-x)*(side==='left'?1:-1);
      const requested=startedCollapsed?minimum+delta-50:start+delta;
      if(side==='left'&&oncollapse){
        if(!isCollapsed&&requested<minimum-50){setCollapsed(true);return;}
        if(isCollapsed){
          if(startedCollapsed?delta<=50:requested<minimum)return;
          setCollapsed(false);
        }
      }
      apply(requested);
    };
    const finish=(next:PointerEvent)=>{if(next.pointerId!==id)return;if(next.type==='pointercancel')restore();else save();cleanup();};
    const cancel=(next:KeyboardEvent)=>{if(next.key==='Escape'){restore();cleanup();}};
    const cleanup=()=>{if(handle.hasPointerCapture(id))handle.releasePointerCapture(id);document.body.style.cursor=cursor;window.removeEventListener('pointermove',move);window.removeEventListener('pointerup',finish);window.removeEventListener('pointercancel',finish);window.removeEventListener('keydown',cancel);window.removeEventListener('blur',blur);activeCleanup=()=>{};};
    const blur=()=>{restore();cleanup();};
    activeCleanup=cleanup;
    window.addEventListener('pointermove',move);window.addEventListener('pointerup',finish);window.addEventListener('pointercancel',finish);window.addEventListener('keydown',cancel);window.addEventListener('blur',blur);
  }
  function keyboard(event:KeyboardEvent){
    if(side==='left'&&oncollapse&&event.key==='Enter'){event.preventDefault();oncollapse(!collapsed);return;}
    if(!['ArrowLeft','ArrowRight','Home','End'].includes(event.key))return;
    event.preventDefault();
    const current=handle.parentElement!.getBoundingClientRect().width;
    if(side==='left'&&collapsed)oncollapse?.(false);
    apply(event.key==='Home'?minimum:event.key==='End'?bounds():current+(event.key==='ArrowRight'?1:-1)*(side==='left'?1:-1)*16);save();
  }
  onMount(()=>{
    try{const stored=Number(localStorage.getItem(key));if(stored>0 && Number.isFinite(stored)){expandedWidth=stored;document.documentElement.style.setProperty(variable,`${stored}px`);}}catch{/* Retain default width. */}
    const observer=new ResizeObserver(()=>{width=handle.parentElement!.getBoundingClientRect().width;maximum=bounds();});observer.observe(handle.parentElement!);
    return ()=>{activeCleanup();observer.disconnect();};
  });
</script>
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (keyboard-operable window splitter) -->
<div bind:this={handle} class="sidebar-resize" class:left={side==='left'} role="separator" tabindex="0" aria-label={`Resize ${side==='left'?'main':'right'} sidebar`} aria-orientation="vertical" aria-valuetext={side==='left'&&collapsed?'Collapsed. Drag outward to expand.':`${Math.round(width)} pixels`} aria-valuemin={Math.min(minimum,maximum,width)} aria-valuemax={Math.round(maximum)} aria-valuenow={Math.round(width)} onpointerdown={drag} ondblclick={fitContent} onkeydown={keyboard}></div>
<style>
  .sidebar-resize{position:absolute;top:0;bottom:0;left:-4px;width:8px;z-index:15;cursor:col-resize;touch-action:none;outline:none}
  .sidebar-resize.left{left:auto;right:-4px}
  .sidebar-resize::after{content:"";position:absolute;top:0;bottom:0;left:3px;width:1px;pointer-events:none}
  .sidebar-resize:hover::after,.sidebar-resize:focus-visible::after{background:var(--accent)}
</style>
