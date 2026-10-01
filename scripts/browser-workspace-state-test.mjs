import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import ts from 'typescript';
const surface=await readFile('src/lib/components/AppSurface.svelte','utf8');
const source=surface.slice(surface.indexOf('>')+1,surface.indexOf('</script>'));
const ast=ts.createSourceFile('surface.ts',source,ts.ScriptTarget.Latest,true);
const names=['suspendBrowserTabs','sanitizePaneState','layoutBrowser'];
const selected=ast.statements.filter(n=>ts.isFunctionDeclaration(n)&&names.includes(n.name?.text));
assert.equal(selected.length,names.length);
const js=ts.transpileModule(selected.map(n=>n.getText(ast).replace(/^export /,'')).join('\n'),{compilerOptions:{target:ts.ScriptTarget.ES2022}}).outputText;
const runtimeSource=await readFile('src/lib/browser-runtime.ts','utf8');
const runtimeJs=ts.transpileModule(runtimeSource.replace(/^import type[^\n]*\n/gm,'').replace(/^export /gm,''),{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.None}}).outputText;
const {claimBrowserViewport}=new Function(runtimeJs+';return {claimBrowserViewport};')();
const oldOwner=claimBrowserViewport('page'),newOwner=claimBrowserViewport('page');
assert.equal(oldOwner.owns(),false,'old pane cannot hide its successor');
oldOwner.release();assert.equal(newOwner.owns(),true);newOwner.release();assert.equal(newOwner.owns(),false);
const liveBrowserTabs=new Set(['page']);
const page={form:'unfinished draft',scroll:540,history:['first','current'],visible:true};
const calls=[];
const bridge={available:true,browserSetLayout:async(id,bounds,visible)=>{calls.push({id,bounds,visible});page.visible=visible;}};
const create=new Function('bridge','liveBrowserTabs',`
let browserTabs={page:{id:'page',url:'https://example.test',title:'Page',unloaded:false}};
const browserBounds={page:{x:0,y:0,width:400,height:300}},browserOpenings=new Map(),browserLayoutRevision=new Map(),browserAppliedLayouts=new Map(),browserClosings=new Set();
const browserLayoutRuntime={tail:Promise.resolve()}; let nativeRuntime=true,activeInterfaceScale=100,embedded=false,active=true,sidebarSelected=false,activePaneId='main',snapshot={tasks:[],projects:[],settings:{showActivePaneBorder:false}},indexes=null,error='',activeWorkspaceKey='all';
const window={innerWidth:1000,innerHeight:800},availableChannels=[],text=String;
const remapTerminalIds=value=>value, normalizeTabOrder=(order)=>order,taskBelongsToWorkspace=()=>true;
const emptyWorkspace=()=>({openTaskIds:[],openDraftIds:[],openChannelIds:[],openTerminalIds:[],openEmptyIds:[],tabOrder:[],taskDrafts:{},drafts:{},queuedAttachments:{},attachmentContexts:{},channelRecipients:{}});
${js}
return {suspendBrowserTabs,sanitizePaneState,layoutBrowser,browserAppliedLayouts,setTabs(value){browserTabs=value}};
`);
let qa=create(bridge,liveBrowserTabs);
const saved={openBrowserIds:['page'],selectedBrowserId:'page',browserTabs:{page:{id:'page',url:'https://example.test',title:'Page',unloaded:false}},pane:'browser'};
await qa.layoutBrowser('page',{x:0,y:0,width:400,height:300,visible:true});
await qa.suspendBrowserTabs();assert.equal(page.visible,false);
// Workspace restoration remounts a new surface but uses the same native child.
qa=create(bridge,liveBrowserTabs);
const restored=qa.sanitizePaneState(saved,{});assert.equal(restored.browserTabs.page.unloaded,false);
qa.setTabs(restored.browserTabs);
await qa.layoutBrowser('page',{x:100,y:40,width:450,height:350,visible:true});
assert.equal(page.visible,true);assert.equal(page.form,'unfinished draft');assert.equal(page.scroll,540);
assert.deepEqual(page.history,['first','current']);assert.equal(calls.at(-1).bounds.x,100);
assert.deepEqual(calls.map(c=>c.visible),[true,false,true]);
// A transferred tab has left the old metadata map before its DOM cleanup.
qa.setTabs({});await qa.layoutBrowser('page',{x:0,y:0,width:0,height:0,visible:false});
assert.equal(page.visible,false,'cleanup hides an orphaned native overlay after metadata transfer');
// Explicit unload/close or process restart has no live child to restore.
liveBrowserTabs.delete('page');assert.equal(qa.sanitizePaneState(saved,{}).browserTabs.page.unloaded,true);
const failed=create({...bridge,browserSetLayout:async()=>{throw Error('hide failed')}},new Set(['page']));
await assert.rejects(failed.suspendBrowserTabs(),/hide failed/);
console.log('workspace browsers: hide/remount/reveal retains live page; fresh bounds, restart/unload and hide failure covered');
