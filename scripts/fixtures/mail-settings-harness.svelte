<script lang="ts">
  import MailSettings from '../../src/lib/components/MailSettings.svelte';
  const local = { id:'local', name:'Local', kind:'local', address:'', user:'', port:22, identityFile:'', defaultCwd:'', codexPath:'', claudePath:'', opencodePath:'', hermesPath:'' };
  const remote = { ...local, id:'remote', kind:'ssh' };
  const base = { description:'', instructions:'', avatar:null, model:'', cwd:'', color:'#3978d4', sandbox:'workspace-write', expertise:[], responsibilities:[], skills:[], collaborationEnabled:false };
  const agents = [
    { ...base, id:'codex', name:'Atlas', provider:'codex', hostId:'local' },
    { ...base, id:'claude', name:'Claude Local', provider:'claude', hostId:'local' },
    { ...base, id:'acp', name:'ACP Local', provider:'acp', hostId:'local', acp:{ command:'example-acp', args:[] } },
    { ...base, id:'agy', name:'AGY', provider:'acp', hostId:'local', acp:{ command:'/opt/bin/monitter-agy-acp', args:[] } },
    { ...base, id:'ssh', name:'Remote Codex', provider:'codex', hostId:'remote' },
    { ...base, id:'internal', name:'Internal', provider:'codex', hostId:'local', internal:true },
    { ...base, id:'other', name:'Other Provider', provider:'gemini', hostId:'local' },
  ];
</script>
<main><MailSettings {agents} hosts={[local, remote]}/></main>
<style>:global(html,body,#app){margin:0;min-height:100%;font-family:Inter,ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;font-size:13px;background:#f4f6f7;color:#19232b;}main{--panel:#fff;--paper:#f7f8f8;--soft:#edf0f0;--line:#ccd4d4;--ink:#162020;--muted:#5d6868;--accent:#3978d4;--accent-ink:#225aa8;--on-accent:#fff;--mono:ui-monospace,monospace;container-type:inline-size;padding:16px;}</style>
