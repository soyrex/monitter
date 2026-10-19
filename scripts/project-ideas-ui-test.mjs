// Isolated browser verification of the actual IdeaListPane; never connects to a backend.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdtemp, readFile, rm, writeFile, mkdir } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { chromium, expect } from '@playwright/test';
const root=process.cwd(), modules=existsSync(join(root,'node_modules'))?join(root,'node_modules'):resolve(root,'../monitter/node_modules');
const temp=await mkdtemp(join(tmpdir(),'monitter-ideas-ui-'));
let server,browser;
const errors=[], results=[];
try {
  await writeFile(join(temp,'Harness.svelte'), `<script>
import IdeaListPane from ${JSON.stringify(join(root,'src/lib/components/IdeaListPane.svelte'))};
let project=$state(window.__ideaProject),visible=$state(true);
window.__ideaHarness={setProject:value=>project=value,toggle:()=>visible=!visible};
</script>
{#if visible}<IdeaListPane {project} draftKey="stable-pane" onsaving={value=>{window.__ideaSaving=value;(window.__ideaSavingEvents??=[]).push(value)}} onUpdate={snapshot=>{project=snapshot.projects[0];window.__ideaProject=project}}/>{/if}
<style>:global(html),:global(body),:global(#app){margin:0;width:100%;height:100%;font-family:system-ui} :global(*){box-sizing:border-box} :global(:root){--ink:#1d2523;--panel:#fff;--line:#c5cfca;--muted:#53625c;--accent:#20774c} :global(html.dark){--ink:#e3ede7;--panel:#17221c;--line:#42554a;--muted:#a3b3aa;--accent:#64b684}</style>`);
  await writeFile(join(temp,'entry.js'), `import { mount } from ${JSON.stringify(join(modules,'svelte/src/index-client.js'))};import Harness from './Harness.svelte';mount(Harness,{target:document.getElementById('app')});`);
  await writeFile(join(temp,'bridge.js'), 'export function getBridge(){return window.__ideaBridge}');
  await writeFile(join(temp,'rolldown.config.mjs'), `import { createRequire } from 'node:module';import compiler from ${JSON.stringify(join(modules,'svelte/compiler/index.js'))};const require=createRequire(${JSON.stringify(join(root,'package.json'))});export default {input:${JSON.stringify(join(temp,'entry.js'))},output:{file:${JSON.stringify(join(temp,'bundle.js'))},format:'es'},plugins:[{name:'focused-svelte',resolveId(id,importer){if(id.startsWith('$lib/')){const file=${JSON.stringify(join(root,'src/lib'))}+'/'+id.slice(5);return ['', '.ts', '.js', '.svelte'].map(ext=>file+ext).find(path=>require('node:fs').existsSync(path));}if(id.startsWith('svelte/'))return require.resolve(id);if(id==='../bridge'&&importer?.endsWith('IdeaListPane.svelte'))return ${JSON.stringify(join(temp,'bridge.js'))};},transform(code,id){if(id.endsWith('.svelte'))return {code:compiler.compile(code,{filename:id,generate:'client',css:'injected'}).js.code,map:null};}}]};`);
  execFileSync(join(modules,'.bin/rolldown'),['--config',join(temp,'rolldown.config.mjs'),'--platform','browser'],{cwd:root,stdio:'inherit'});
  server=createServer(async(req,res)=>{if(req.url==='/bundle.js'){res.setHeader('content-type','text/javascript');res.end(await readFile(join(temp,'bundle.js')));}else res.end('<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1"><div id="app"></div>');});
  await new Promise(ok=>server.listen(0,'127.0.0.1',ok));
  browser=await chromium.launch({headless:true,...(process.env.MONITTER_CHROME_PATH?{executablePath:process.env.MONITTER_CHROME_PATH}:{})});
  await mkdir(join(root,'verification'),{recursive:true});
  for(const width of [320,390,1280]) for(const theme of ['light','dark']) {
    const page=await browser.newPage({viewport:{width,height:844}});page.on('pageerror',e=>{errors.push(e.message);console.error('Idea fixture browser error:',e.message)});
    await page.addInitScript(()=>{
      window.__ideaCalls=[];window.__ideaFail=false;window.__ideaHold=false;window.__ideaSavingEvents=[];
      window.__ideaProject={id:'qa',name:'QA',description:'',icon:'folder',color:'#20774c',workspaces:[],ideasRevision:7,ideas:[{id:'seed',title:'Existing idea',markdown:'## Existing document',status:'open',tags:['seed'],createdAt:1,updatedAt:2,authorKind:'agent',authorName:'Atlas',history:[{editedAt:1,editedBy:{kind:'person',name:'You',id:null},title:'Prior title',markdown:'Old details',status:'open',tags:[]}]}]};
      window.__ideaBridge={saveProjectIdeas:async(projectId,ideas,expectedRevision)=>{window.__ideaCalls.push({projectId,ideas:JSON.parse(JSON.stringify(ideas)),expectedRevision});if(window.__ideaHold)await new Promise(resolve=>window.__ideaRelease=resolve);if(window.__ideaFail)throw Error('Simulated save failure');if(expectedRevision!==window.__ideaProject.ideasRevision)throw Error('The Idea List changed elsewhere. Refresh before saving.');window.__ideaProject={...window.__ideaProject,ideas,ideasRevision:expectedRevision+1};return {projects:[window.__ideaProject]};}};
    });
    await page.goto(`http://127.0.0.1:${server.address().port}`);await page.evaluate(theme=>{document.documentElement.classList.toggle('dark',theme==='dark');return import('/bundle.js')},theme);
    await page.getByRole('button',{name:'Existing idea',exact:false}).click();
    await expect(page.getByRole('heading',{name:'Existing document'})).toBeVisible();
    await page.getByText('Edit history (1)',{exact:true}).click();await expect(page.getByText('Prior title',{exact:true})).toBeVisible();
    await page.getByRole('button',{name:'Capture idea'}).click();
    await page.getByLabel('Idea title',{exact:true}).fill('Captured idea');await page.getByLabel('Idea Markdown').fill('## Draft preview\n\n**Bold text**');
    await page.getByRole('combobox',{name:/^Status/}).selectOption('inPlan');await page.getByLabel('Idea tags').fill('mobile, follow-up');
    await page.getByRole('button',{name:'Preview',exact:true}).click();await expect(page.getByRole('heading',{name:'Draft preview'})).toBeVisible();await expect(page.locator('.preview strong')).toHaveText('Bold text');
    // Real component unmount/remount, with the same pane identity.
    await page.evaluate(()=>window.__ideaHarness.toggle());await expect(page.getByRole('button',{name:'Capture idea'})).toHaveCount(0);await page.evaluate(()=>window.__ideaHarness.toggle());
    await expect(page.getByLabel('Idea title',{exact:true})).toHaveValue('Captured idea');await expect(page.getByLabel('Idea Markdown')).toHaveValue('## Draft preview\n\n**Bold text**');
    await page.evaluate(()=>window.__ideaFail=true);await page.getByRole('button',{name:'Save idea',exact:true}).click();await expect(page.getByRole('alert')).toContainText('Simulated save failure');await expect(page.getByLabel('Idea title',{exact:true})).toHaveValue('Captured idea');
    await page.evaluate(()=>{window.__ideaFail=false;window.__ideaHold=true});await page.getByRole('button',{name:'Save idea',exact:true}).click();
    await expect.poll(()=>page.evaluate(()=>window.__ideaSaving)).toBe(true);
    await expect(page.getByRole('button',{name:'Existing idea',exact:false})).toBeDisabled();
    const pendingCalls=await page.evaluate(()=>window.__ideaCalls.length);
    await page.evaluate(()=>window.__ideaHarness.toggle());await expect(page.getByRole('button',{name:'Capture idea'})).toHaveCount(0);await page.evaluate(()=>window.__ideaHarness.toggle());
    await expect(page.getByRole('button',{name:'Saving…',exact:true})).toBeDisabled();
    await expect(page.getByLabel('Idea title',{exact:true})).toHaveValue('Captured idea');
    assert.equal(await page.evaluate(()=>window.__ideaCalls.length),pendingCalls,'remount must not submit a second mutation');
    await page.evaluate(()=>window.__ideaRelease());
    await expect.poll(()=>page.evaluate(()=>window.__ideaSaving)).toBe(false);
    await expect(page.getByLabel('Idea title',{exact:true})).toHaveCount(0);
    await page.evaluate(()=>window.__ideaHarness.toggle());await expect(page.getByRole('button',{name:'Capture idea'})).toHaveCount(0);await page.evaluate(()=>window.__ideaHarness.toggle());
    await expect(page.getByLabel('Idea title',{exact:true})).toHaveCount(0);
    const call=await page.evaluate(()=>window.__ideaCalls.at(-1));assert.equal(call.expectedRevision,7);assert.equal(call.projectId,'qa');assert.equal(call.ideas.at(-1).status,'inPlan');assert.deepEqual(call.ideas.at(-1).tags,['mobile','follow-up']);
    await page.getByRole('button',{name:'Captured idea',exact:false}).click();await page.getByLabel('Idea title',{exact:true}).fill('Retained conflicting draft');
    await page.evaluate(()=>{window.__ideaProject={...window.__ideaProject,ideasRevision:9};window.__ideaHarness.setProject(window.__ideaProject)});
    await expect(page.getByRole('alert')).toContainText('Your draft is retained');await expect(page.getByLabel('Idea title',{exact:true})).toHaveValue('Retained conflicting draft');await expect(page.getByRole('button',{name:'Save idea',exact:true})).toBeDisabled();
    const geometry=await page.evaluate(()=>({overflow:document.documentElement.scrollWidth>innerWidth,small:[...document.querySelectorAll('.ideas button,.ideas input,.ideas select,.ideas summary')].filter(el=>el.getBoundingClientRect().height>0&&el.getBoundingClientRect().height<43.5).map(el=>el.textContent)}));
    assert.equal(geometry.overflow,false,`${width}/${theme}: horizontal overflow`);if(width<=390)assert.deepEqual(geometry.small,[],`${width}/${theme}: controls below 44px`);
    await page.screenshot({path:join(root,`verification/ideas-${width}-${theme}.png`),fullPage:true});results.push({width,theme,passed:true});await page.close();
  }
  assert.deepEqual(errors,[]);await writeFile(join(root,'verification/project-ideas-ui-results.json'),JSON.stringify({results,errors},null,2));console.log(JSON.stringify(results));
} finally {await browser?.close();if(server)await new Promise(ok=>server.close(ok));await rm(temp,{recursive:true,force:true});}
