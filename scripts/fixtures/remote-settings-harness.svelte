<script lang="ts">
  import { onMount } from 'svelte';
  import RemoteControl from '../../src/lib/components/RemoteControl.svelte';
  import SettingsPane from '../../src/lib/components/SettingsPane.svelte';
  import { remoteControlOpen } from '../../src/lib/workspace-panels';

  let mounted = $state(true);
  let category = $state('remote');
  const settings = { accent:'#3978d4', theme:'light', interfaceScale:100, showToolActivity:true, showReasoningSummaries:true, sendWithEnter:false, sidebarView:'standard' };
  onMount(() => { (window as any).__REMOTE_SETTINGS_QA__ = { unmount:()=>mounted=false, mount:()=>mounted=true, category:(value:string)=>category=value }; });
</script>

<main>
  <RemoteControl bind:open={$remoteControlOpen}/>
  {#if mounted}<SettingsPane {settings} bind:category onsave={async()=>{}} />{/if}
</main>

<style>
  :global(html,body,#app){margin:0;height:100%;} :global(*){box-sizing:border-box;} main{--panel:#fff;--paper:#f7f8f8;--soft:#edf0f0;--line:#ccd4d4;--ink:#162020;--muted:#5d6868;--accent:#3978d4;--accent-ink:#225aa8;--on-accent:#fff;--mono:ui-monospace,monospace;container-type:inline-size;height:100vh;}
</style>
