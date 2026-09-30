<script>
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
<div class="host">
<PaneGrid {layout} activePaneId="browser" onactivate={()=>{}} onresize={(id,ratio)=>layout={...layout,ratio}} ondropTab={()=>{}}>
  {#snippet children(id)}
    {#if id==='browser'}<div class="fill"><BrowserTabPanel tab={{id:'test',url:'https://example.test',title:'Test',unloaded:false}} active={true} layoutRevision={revision} onLayout={rect=>window.paneQA.reports.push(rect)}/></div>{:else}<div>Other pane</div>{/if}
  {/snippet}
</PaneGrid>
</div>
<style>.host{display:flex;width:900px;height:620px}.fill{flex:1;min-width:0;min-height:0}</style>
