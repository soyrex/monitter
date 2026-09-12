/** Pointer sorting stays inside the webview; it never starts a native file drag. */
export function sidebarReorder(node: HTMLElement, initial: {group:string; id:string; move:(group:string,id:string,target:string,after:boolean)=>void}) {
  let config=initial;
  let cleanup=()=>{};
  const attributes=()=>{node.dataset.sidebarSortGroup=config.group;node.dataset.sidebarSortId=config.id;};
  attributes();
  const nativeDrag=(event:DragEvent)=>{if(config.group)event.preventDefault();};
  const down=(event:PointerEvent)=>{
    // Reordering is mouse-only. On touch, a little movement while tapping or
    // scrolling could cross the drag threshold and suppress the native click.
    if(event.pointerType!=='mouse' || !config.group || event.button!==0 || !(event.target instanceof Element) || event.target.closest('.quiet,.chat-actions,.folder-toggle'))return;
    cleanup();
    const x=event.clientX,y=event.clientY,pointer=event.pointerId,previousCursor=document.body.style.cursor;
    let dragging=false,target:HTMLElement|null=null,after=false;
    const clear=()=>{target?.removeAttribute('data-sort-edge');target=null;};
    const move=(next:PointerEvent)=>{
      if(next.pointerId!==pointer)return;
      if(!dragging && Math.hypot(next.clientX-x,next.clientY-y)<6)return;
      if(!dragging){dragging=true;node.setPointerCapture(pointer);node.dataset.sortDragging='true';document.body.style.cursor='grabbing';}
      next.preventDefault();clear();
      target=document.elementsFromPoint(next.clientX,next.clientY).map(el=>el.closest<HTMLElement>('[data-sidebar-sort-group]')).find(el=>el && el!==node && el.dataset.sidebarSortGroup===config.group)??null;
      if(target){const box=target.getBoundingClientRect();after=next.clientY>box.top+box.height/2;target.dataset.sortEdge=after?'after':'before';}
      const scroll=node.closest<HTMLElement>('.side-scroll,.agent-rail,.rail-chat-list');
      if(scroll){const box=scroll.getBoundingClientRect();if(next.clientY<box.top+30)scroll.scrollTop-=12;else if(next.clientY>box.bottom-30)scroll.scrollTop+=12;}
    };
    const finish=(next:PointerEvent|KeyboardEvent)=>{
      if(next instanceof KeyboardEvent && next.key!=='Escape')return;
      const destination=target?.dataset.sidebarSortId;
      if(next.type==='pointerup' && dragging && destination)config.move(config.group,config.id,destination,after);
      if(dragging){const suppress=(click:MouseEvent)=>{click.preventDefault();click.stopImmediatePropagation();};window.addEventListener('click',suppress,true);setTimeout(()=>window.removeEventListener('click',suppress,true),0);}
      cleanup();
    };
    cleanup=()=>{clear();delete node.dataset.sortDragging;document.body.style.cursor=previousCursor;if(node.hasPointerCapture(pointer))node.releasePointerCapture(pointer);window.removeEventListener('pointermove',move);window.removeEventListener('pointerup',finish);window.removeEventListener('pointercancel',finish);window.removeEventListener('keydown',finish);window.removeEventListener('blur',cancel);cleanup=()=>{};};
    const cancel=()=>cleanup();
    window.addEventListener('blur',cancel);
    window.addEventListener('pointermove',move,{passive:false});window.addEventListener('pointerup',finish);window.addEventListener('pointercancel',finish);window.addEventListener('keydown',finish);
  };
  node.addEventListener('pointerdown',down);node.addEventListener('dragstart',nativeDrag);
  return {update(next:typeof initial){config=next;attributes();},destroy(){cleanup();node.removeEventListener('pointerdown',down);node.removeEventListener('dragstart',nativeDrag);}};
}
