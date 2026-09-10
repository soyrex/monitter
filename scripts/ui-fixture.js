// Browser QA only. This file is never imported into the shipped application.
(() => {
  const local = { id:'local', name:'This Mac', kind:'local', address:'', user:'', port:0, identityFile:'', defaultCwd:'/tmp/monitter-ui-test', codexPath:'codex', claudePath:'', opencodePath:'', hermesPath:'' };
  let state = {
    hosts:[local],
    agents:[{id:'atlas',avatar:null,name:'Atlas',description:'Coding partner',instructions:'Work carefully and explain the result.',provider:'codex',model:'',hostId:'local',cwd:local.defaultCwd,color:'#397e61',sandbox:'read-only',expertise:[],responsibilities:[],skills:[],collaborationEnabled:true}],
    tasks:[],messages:[],events:[],channels:[],projects:[],collaborations:[],settings:{accent:'#3f9d6a',theme:'light',interfaceScale:125,showToolActivity:true,showReasoningSummaries:true,sendWithEnter:false,sidebarView:'standard'}
  };
  const clone = value => JSON.parse(JSON.stringify(value));
  const callbacks = new Set();
  const calls = [];
  const copy = () => clone(state);
  const notify = () => callbacks.forEach(fn => fn());
  const record = (method,args) => calls.push({method,args});
  const save = (collection,item) => {
    item = {...item,id:item.id || crypto.randomUUID()};
    const i=state[collection].findIndex(x=>x.id===item.id);
    if(i<0) state[collection].push(item); else state[collection][i]=item;
    notify(); return copy();
  };
  const createTask = async input => {
    record('createTask',input);
    const a=state.agents.find(a=>a.id===input.agentId);
    const project=state.projects.find(p=>p.id===input.projectId);
    const task={id:crypto.randomUUID(),agentId:a.id,title:input.title,nativeSessionId:input.nativeSessionId||null,status:'idle',archived:false,createdAt:Date.now(),updatedAt:Date.now(),parentTaskId:input.parentTaskId||null,channelId:input.channelId||null,projectId:input.projectId||null,hostId:a.hostId,cwd:project?.workspaces.find(w=>w.hostId===a.hostId)?.cwd || a.cwd,provider:a.provider,model:a.model,sandbox:a.sandbox};
    state.tasks.push(task);notify();return clone(task);
  };
  window.__MONITTER_QA__ = { calls, snapshot:copy, emit:notify, setSnapshot:s=>{state=clone(s);notify();} };
  window.__MONITTER_BRIDGE__ = {
    available:true, getSnapshot:async()=>copy(),
    saveHost:async h=>{record('saveHost',h);return save('hosts',h);},
    deleteHost:async id=>{record('deleteHost',id);state.hosts=state.hosts.filter(h=>h.id!==id);notify();return copy();},
    probeHost:async h=>{record('probeHost',h);return {ok:true,versions:{codex:'QA fixture version'},message:'Connection result from the browser test fixture.'};},
    saveAgent:async a=>{record('saveAgent',a);return save('agents',a);},
    deleteAgent:async id=>{record('deleteAgent',id);state.agents=state.agents.filter(a=>a.id!==id);notify();return copy();},
    createTask,
    renameTask:async(id,title)=>{record('renameTask',{id,title});state.tasks.find(t=>t.id===id).title=title;notify();return copy();},
    deleteTask:async id=>{record('deleteTask',id);state.tasks=state.tasks.filter(t=>t.id!==id);notify();return copy();},
    setTaskArchived:async(taskId,archived)=>{record('setTaskArchived',{taskId,archived});state.tasks.find(t=>t.id===taskId).archived=archived;notify();return copy();},
    saveProject:async project=>{record('saveProject',project);return save('projects',project);},
    deleteProject:async id=>{record('deleteProject',id);state.projects=state.projects.filter(p=>p.id!==id);state.tasks.forEach(t=>{if(t.projectId===id)t.projectId=null;});notify();return copy();},
    setTaskProject:async(taskId,projectId)=>{record('setTaskProject',{taskId,projectId});state.tasks.find(t=>t.id===taskId).projectId=projectId;notify();return copy();},
    sendMessage:async(taskId,text)=>{
      record('sendMessage',{taskId,text});
      if(text==='TEST_FAILURE') throw Error('Deliberate QA transport failure');
      state.tasks.find(t=>t.id===taskId).status='running';
      state.messages.push({id:crypto.randomUUID(),taskId,role:'user',text,createdAt:Date.now()});
      state.events.push({id:crypto.randomUUID(),taskId,kind:'status',title:'Running',detail:'Browser test event; no agent is being executed.',createdAt:Date.now()});
      notify();return copy();
    },
    cancelTask:async taskId=>{record('cancelTask',taskId);state.tasks.find(t=>t.id===taskId).status='interrupted';notify();return copy();},
    saveSettings:async settings=>{record('saveSettings',settings);state.settings=settings;notify();return copy();},
    saveChannel:async c=>{record('saveChannel',c);return save('channels',c);},
    sendChannelMessage:async(channelId,text,agentIds)=>{record('sendChannelMessage',{channelId,text,agentIds});state.channels.find(c=>c.id===channelId).messages.push({id:crypto.randomUUID(),role:'user',agentId:null,text,createdAt:Date.now(),taskId:null});notify();return copy();},
    getTaskGoal:async taskId=>{record('getTaskGoal',taskId);return clone(state.goals?.[taskId] ?? null);},
    getResumeCommand:async taskId=>{record('getResumeCommand',taskId);return "codex resume 'QA-session'";},
    onChanged:async handler=>{callbacks.add(handler);return ()=>callbacks.delete(handler);}
  };
})();
