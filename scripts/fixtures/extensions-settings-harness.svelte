<script lang="ts">
  import ExtensionsSettings from '../../src/lib/components/ExtensionsSettings.svelte';
  const agents = [{ id:'agent-1', name:'Atlas', description:'', instructions:'', avatar:null, provider:'codex', model:'', hostId:'', cwd:'', color:'#3978d4', sandbox:'workspace-write', expertise:[], responsibilities:[], skills:[], collaborationEnabled:false }];
</script>
<main><ExtensionsSettings {agents}/></main>
<style>:global(html,body,#app){margin:0;min-height:100%;}main{--panel:#fff;--paper:#f7f8f8;--soft:#edf0f0;--line:#ccd4d4;--ink:#162020;--muted:#5d6868;--accent:#3978d4;--accent-ink:#225aa8;--on-accent:#fff;--mono:ui-monospace,monospace;container-type:inline-size;padding:16px;}</style>
