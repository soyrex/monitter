// Browser QA only. This file is never imported into the shipped application.
(() => {
  const local = { id:'local', name:'This Mac', kind:'local', address:'', user:'', port:0, identityFile:'', defaultCwd:'/tmp/monitter-ui-test', codexPath:'codex', claudePath:'', opencodePath:'', hermesPath:'' };
  let state = {
    hosts:[local],
    agents:[{id:'atlas',avatar:null,name:'Atlas',description:'Coding partner',instructions:'Work carefully and explain the result.',provider:'codex',model:'',hostId:'local',cwd:local.defaultCwd,color:'#397e61',sandbox:'read-only',expertise:[],responsibilities:[],skills:[],collaborationEnabled:true}],
    tasks:[],messages:[],events:[],channels:[],projects:[],collaborations:[],queuedMessages:[],settings:{accent:'#3f9d6a',theme:'light',interfaceScale:125,showToolActivity:true,showReasoningSummaries:true,sendWithEnter:false,sidebarView:'standard'}
  };
  const clone = value => JSON.parse(JSON.stringify(value));
  const callbacks = new Set();
  const calls = [];
  const attachments = new Map();
  const terminals = new Map();
  const terminalBytes = text => Array.from(new TextEncoder().encode(text));
  const terminalTarget = target => {
    const task = state.tasks.find(task => task.id === target.taskId);
    const agent = state.agents.find(agent => agent.id === (target.agentId || task?.agentId));
    const host = state.hosts.find(host => host.id === (target.hostId || task?.hostId || agent?.hostId)) || (!target.hostId && !task && !agent ? state.hosts.find(host=>host.kind==='local') : undefined);
    const project = state.projects.find(project => project.id === target.projectId);
    if (!host) throw Error('Terminal host was not found.');
    return { host, cwd: target.cwd || task?.cwd || project?.workspaces?.find(workspace => workspace.hostId === host.id)?.cwd || agent?.cwd || host.defaultCwd };
  };
  const terminalChunk = (terminal, data) => terminal.chunks.push({ seq: terminal.nextSeq++, data: terminalBytes(data) });
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
    const task={id:crypto.randomUUID(),agentId:a.id,title:input.title,nativeSessionId:input.nativeSessionId||null,status:'idle',archived:false,createdAt:Date.now(),updatedAt:Date.now(),parentTaskId:input.parentTaskId||null,channelId:input.channelId||null,projectId:input.projectId||null,hostId:a.hostId,cwd:project?.workspaces.find(w=>w.hostId===a.hostId)?.cwd || a.cwd,provider:a.provider,model:input.modelSettings?.model || a.model,modelSettings:input.modelSettings || null,sandbox:a.sandbox};
    state.tasks.push(task);notify();return clone(task);
  };
  window.__MONITTER_QA__ = { calls, snapshot:copy, emit:notify, setSnapshot:s=>{state=clone(s);notify();}, terminals:()=>clone([...terminals.values()]), terminalOutput:(id,data)=>{const terminal=terminals.get(id);if(!terminal)throw Error('Terminal was not found.');terminalChunk(terminal,data);}, terminalCloseFailure:null };
  window.__MONITTER_BRIDGE__ = {
    available:true, getSnapshot:async()=>copy(),
    setChannelMembership:async(channelId,agentId,member)=>{record('setChannelMembership',{channelId,agentId,member});const c=state.channels.find(c=>c.id===channelId);if(!c||!state.agents.some(a=>a.id===agentId))throw Error('Channel or agent was not found.');c.agentIds=member?[...new Set([...c.agentIds,agentId])]:c.agentIds.filter(id=>id!==agentId);if(!member)state.tasks.filter(t=>t.channelId===channelId&&t.agentId===agentId&&t.status==='running').forEach(t=>t.status='interrupted');notify();return copy();},
    saveHost:async h=>{record('saveHost',h);return save('hosts',h);},
    deleteHost:async id=>{record('deleteHost',id);state.hosts=state.hosts.filter(h=>h.id!==id);notify();return copy();},
    probeHost:async h=>{record('probeHost',h);return {ok:true,versions:{codex:'QA fixture version'},message:'Connection result from the browser test fixture.'};},
    saveAgent:async a=>{record('saveAgent',a);return save('agents',a);},
    deleteAgent:async id=>{record('deleteAgent',id);state.agents=state.agents.filter(a=>a.id!==id);notify();return copy();},
    createTask,
    renameTask:async(id,title)=>{record('renameTask',{id,title});state.tasks.find(t=>t.id===id).title=title;notify();return copy();},
    autoname:async target=>{record('autoname',{target});const title=target.taskId?'Generated chat title':target.channelId?'Generated channel title':'Generated terminal title';if(target.taskId){const task=state.tasks.find(t=>t.id===target.taskId);if(!task)throw Error('Task was not found.');task.title=title;}else if(target.channelId){const channel=state.channels.find(c=>c.id===target.channelId);if(!channel)throw Error('Channel was not found.');channel.name=title;}else if(target.terminalId){const terminal=terminals.get(target.terminalId);if(!terminal)throw Error('Terminal session was not found.');terminal.title=title;}else throw Error('Choose one pane to name.');notify();return copy();},
    deleteTask:async id=>{record('deleteTask',id);state.tasks=state.tasks.filter(t=>t.id!==id);notify();return copy();},
    previewTaskDeletion:async taskId=>{record('previewTaskDeletion',{taskId});return clone(state.deletionPreview?.[taskId] ?? {supported:false,reason:'Native session files are not available for this task.',files:[]});},
    deleteArchivedTask:async(taskId,removeNativeFiles)=>{record('deleteArchivedTask',{taskId,removeNativeFiles});if(state.deleteFailure?.[taskId]) throw Error(state.deleteFailure[taskId]);const task=state.tasks.find(t=>t.id===taskId);if(!task?.archived) throw Error('Only archived chats can be permanently deleted.');state.tasks=state.tasks.filter(t=>t.id!==taskId);state.messages=state.messages.filter(m=>m.taskId!==taskId);state.events=state.events.filter(e=>e.taskId!==taskId);notify();return copy();},
    setTaskArchived:async(taskId,archived)=>{record('setTaskArchived',{taskId,archived});state.tasks.find(t=>t.id===taskId).archived=archived;notify();return copy();},
    saveProject:async project=>{record('saveProject',project);return save('projects',project);},
    deleteProject:async id=>{record('deleteProject',id);state.projects=state.projects.filter(p=>p.id!==id);state.tasks.forEach(t=>{if(t.projectId===id)t.projectId=null;});notify();return copy();},
    setTaskProject:async(taskId,projectId)=>{record('setTaskProject',{taskId,projectId});state.tasks.find(t=>t.id===taskId).projectId=projectId;notify();return copy();},
    sendMessage:async(taskId,text,attachmentIds=[])=>{
      record('sendMessage',{taskId,text,attachmentIds});
      if(text==='TEST_FAILURE') throw Error('Deliberate QA transport failure');
      if(state.tasks.find(t=>t.id===taskId).status==='running'){state.queuedMessages??=[];state.queuedMessages.push({id:crypto.randomUUID(),taskId,channelId:null,text,attachmentIds,createdAt:Date.now(),status:'queued',error:null});notify();return copy();}
      state.tasks.find(t=>t.id===taskId).status='running';
      state.messages.push({id:crypto.randomUUID(),taskId,role:'user',text,createdAt:Date.now(),attachments:attachmentIds.map(id=>clone(attachments.get(id)))});
      state.events.push({id:crypto.randomUUID(),taskId,kind:'status',title:'Running',detail:'Browser test event; no agent is being executed.',createdAt:Date.now()});
      notify();return copy();
    },
    editQueuedMessage:async (id,text)=>{record('editQueuedMessage',{id,text});const message=state.queuedMessages.find(m=>m.id===id);if(!message)throw Error('Queued message was not found.');if(message.status==='sending')throw Error('Message is already being sent.');if(message.origin==='channel-agent-mention')throw Error('Agent requests cannot be rewritten.');if(!text.trim()&&!message.attachmentIds.length)throw Error('Enter a message or keep an attachment.');message.text=text;notify();return copy();},
    cancelQueuedMessage:async id=>{record('cancelQueuedMessage',{id});state.queuedMessages=(state.queuedMessages??[]).filter(m=>m.id!==id);notify();return copy();},
    cancelTask:async taskId=>{record('cancelTask',taskId);state.tasks.find(t=>t.id===taskId).status='interrupted';notify();return copy();},
    saveSettings:async settings=>{record('saveSettings',settings);state.settings=settings;notify();return copy();},
    setChannelAgentConversation:async(channelId,enabled,turnLimit)=>{record('setChannelAgentConversation',{channelId,enabled,turnLimit});if(!Number.isInteger(turnLimit)||turnLimit<1||turnLimit>20)throw Error('Invalid turn limit');const channel=state.channels.find(c=>c.id===channelId);Object.assign(channel,{agentConversationEnabled:enabled,agentConversationTurnLimit:turnLimit,agentConversationPaused:false});notify();return copy();},
    stopChannelAgentConversation:async channelId=>{record('stopChannelAgentConversation',{channelId});const channel=state.channels.find(c=>c.id===channelId);channel.agentConversationPaused=true;for(const task of state.tasks.filter(t=>t.channelId===channelId&&t.status==='running'))task.status='interrupted';notify();return copy();},
    saveChannel:async c=>{record('saveChannel',c);return save('channels',c);},
    sendChannelMessage:async(channelId,text,agentIds,attachmentIds=[])=>{record('sendChannelMessage',{channelId,text,agentIds,attachmentIds});const channel=state.channels.find(c=>c.id===channelId);channel.agentConversationTurnsUsed=0;channel.agentConversationPaused=false;for(const agentId of agentIds){const task=state.tasks.find(t=>t.channelId===channelId&&t.agentId===agentId&&t.status==='running');if(task){state.queuedMessages??=[];state.queuedMessages.push({id:crypto.randomUUID(),taskId:task.id,channelId,text,attachmentIds,createdAt:Date.now(),status:'queued',error:null});}}state.channels.find(c=>c.id===channelId).messages.push({id:crypto.randomUUID(),role:'user',agentId:null,text,createdAt:Date.now(),taskId:null,attachments:attachmentIds.map(id=>clone(attachments.get(id)))});notify();return copy();},
    getTaskGitStatus:async taskId=>{record('getTaskGitStatus',{taskId});return clone(state.gitStatus?.[taskId] ?? {repository:false});},
    waitForTaskGitMarker:async()=> 'already-present',
    getTaskGitDiff:async(taskId,path,scope)=>{record('getTaskGitDiff',{taskId,path,scope});return clone(state.gitDiffs?.[taskId]?.[`${scope}:${path}`] ?? {repository:false});},
    getTaskGoal:async taskId=>{record('getTaskGoal',taskId);return clone(state.goals?.[taskId] ?? null);},
    resumeTask:async taskId=>{record('resumeTask',{taskId});const task=state.tasks.find(t=>t.id===taskId);if(state.resumeFailure)throw Error(state.resumeFailure);if(!task?.nativeSessionId || task.archived || task.status==='running')throw Error('This session cannot be resumed.');task.status='running';state.messages.push({id:crypto.randomUUID(),taskId,role:'user',text:'Continue from where we left off. If the last request is complete, let me know and wait for my next instruction.',createdAt:Date.now()});notify();return copy();},
    getModelCatalog:async target=>{
      record('getModelCatalog',target);if(state.modelCatalogFailure)throw Error(state.modelCatalogFailure);
      const task=state.tasks.find(t=>t.id===target.taskId),agent=state.agents.find(a=>a.id===(target.agentId||task?.agentId));
      return clone(state.modelCatalog || {models:[{id:'qa-balanced',name:'QA Balanced',description:'Model catalog fixture',reasoningEfforts:[{id:'low',description:'Lower effort'},{id:'medium',description:'Balanced effort'},{id:'high',description:'Higher effort'}],defaultEffort:'medium',supportsFast:true,fastDescription:'Fixture faster responses, increased usage'},{id:'qa-simple',name:'QA Simple',description:'No effort or fast support',reasoningEfforts:[],defaultEffort:null,supportsFast:false,fastDescription:null}],current:task?.modelSettings || {model:task?.model || agent?.model || 'qa-balanced',reasoningEffort:'medium',fastMode:false},source:'Browser QA fixture',warning:null});
    },
    setTaskModelSettings:async(taskId,settings)=>{record('setTaskModelSettings',{taskId,settings});const task=state.tasks.find(t=>t.id===taskId);if(task.status==='running')throw Error('Wait for the current run to finish.');task.model=settings.model;task.modelSettings=settings.model?clone(settings):null;notify();return copy();},
    setTaskSandbox:async(taskId,sandbox)=>{record('setTaskSandbox',{taskId,sandbox});const task=state.tasks.find(t=>t.id===taskId);if(task.status==='running')throw Error('Wait for the current run to finish before changing permissions.');task.sandbox=sandbox;notify();return copy();},
    listTerminals:async()=>{record('listTerminals',{});return clone([...terminals.values()].map(({cols,rows,nextSeq,chunks,writes,...session})=>session));},
    openTerminal:async(target,cols,rows)=>{
      record('openTerminal',{target,cols,rows});const {host,cwd}=terminalTarget(target);const session={id:crypto.randomUUID(),title:`${host.name} shell`,hostId:host.id,cwd,status:'running',exitCode:null};
      const terminal={...session,cols,rows,nextSeq:1,chunks:[],writes:[]};terminalChunk(terminal,`Monitter browser terminal: ${cwd}\r\n$ `);terminals.set(session.id,terminal);return clone(session);
    },
    writeTerminal:async(id,data)=>{record('writeTerminal',{id,data});const terminal=terminals.get(id);if(!terminal)throw Error('Terminal was not found.');if(terminal.status!=='running')throw Error('Terminal has exited.');terminal.writes.push(data);if(data==='\u0004'){terminal.status='exited';terminal.exitCode=0;terminalChunk(terminal,'logout\r\n');return;}terminalChunk(terminal,data==='\u0003'?'^C\r\n$ ':data==='\u0010'?'^P\r\n$ ':data);return undefined;},
    resizeTerminal:async(id,cols,rows)=>{record('resizeTerminal',{id,cols,rows});const terminal=terminals.get(id);if(!terminal)throw Error('Terminal was not found.');terminal.cols=cols;terminal.rows=rows;},
    readTerminal:async(id,afterSeq)=>{record('readTerminal',{id,afterSeq});const terminal=terminals.get(id);if(!terminal)throw Error('Terminal was not found.');return clone({chunks:terminal.chunks.filter(chunk=>chunk.seq>afterSeq).slice(0,128),nextSeq:terminal.nextSeq-1,status:terminal.status,exitCode:terminal.exitCode,truncated:false});},
    closeTerminal:async id=>{record('closeTerminal',{id});const terminal=terminals.get(id);if(!terminal)throw Error('Terminal was not found.');if(window.__MONITTER_QA__.terminalCloseFailure)throw Error(window.__MONITTER_QA__.terminalCloseFailure);terminal.status='exited';terminal.exitCode=0;terminals.delete(id);},
    readAttachmentFile:async sourcePath=>{record('readAttachmentFile',{sourcePath});return {filename:sourcePath.split('/').at(-1),mimeType:'text/plain',dataBase64:btoa('Native file fixture')};},
    storeAttachment:async(target,file,previewDataUrl=null,sourceId)=>{
      record('storeAttachment',{target,...file,previewDataUrl,sourceId});
      if(state.attachmentFailure)throw Error(state.attachmentFailure);
      const task=state.tasks.find(t=>t.id===target.taskId),agent=state.agents.find(a=>a.id===target.agentId),project=state.projects.find(p=>p.id===target.projectId);
      const cwd=task?.cwd || project?.workspaces.find(w=>w.hostId===agent?.hostId)?.cwd || agent?.cwd;
      const item={id:crypto.randomUUID(),name:file.filename,mimeType:file.mimeType,size:atob(file.dataBase64).length,path:`${cwd}/.monitter/attachments/${crypto.randomUUID()}-${file.filename}`,previewDataUrl,sourceId};attachments.set(item.id,item);return clone(item);
    },
    onChanged:async handler=>{callbacks.add(handler);return ()=>callbacks.delete(handler);}
  };
})();
