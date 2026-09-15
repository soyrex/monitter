import {chromium,expect} from '@playwright/test';
import {readFileSync} from 'node:fs';
const browser=await chromium.launch({headless:true});
try {
 const page=await browser.newPage({viewport:{width:1450,height:1000}});
 await page.addInitScript({content:readFileSync('scripts/ui-fixture.js','utf8')+`const q=window.__MONITTER_QA__,s=q.snapshot();s.agents=[{...s.agents[0],id:'rafa',name:'Rafa'},{...s.agents[0],id:'justine',name:'Justine'}];s.channels=[{id:'room',name:'Team',description:'',agentIds:['rafa','justine'],messages:[]}];q.setSnapshot(s);`});
 await page.goto('http://127.0.0.1:18433');await expect(page.getByRole('tab',{name:'Agents view',exact:true})).toBeEnabled({timeout:60000});await page.getByRole('complementary',{name:'Agents and tasks'}).getByRole('button',{name:/Team/}).click();
 const members=page.getByRole('complementary',{name:'Channel members'}),toggle=members.getByRole('switch',{name:'Agent conversation',exact:true});await expect(toggle).not.toBeChecked();await toggle.check();
 const limit=members.getByRole('spinbutton',{name:'Automatic reply limit'});await expect(limit).toHaveValue('6');await limit.fill('4');await limit.press('Tab');await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.snapshot().channels[0].agentConversationTurnLimit)).toBe(4);
 await page.evaluate(()=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.channels[0].agentConversationTurnsUsed=3;s.tasks.push({id:'run',agentId:'rafa',channelId:'room',title:'Peer conversation',status:'running',hostId:'local',cwd:'/tmp',provider:'codex',model:'',sandbox:'read-only',archived:false,createdAt:1,updatedAt:1});q.setSnapshot(s);});
 await expect(members.getByRole('status')).toContainText('3 of 4');await members.getByRole('button',{name:'Stop agent conversation',exact:true}).click();await expect(members.getByRole('status')).toContainText('Paused');expect(await page.evaluate(()=>window.__MONITTER_QA__.snapshot().tasks[0].status)).toBe('interrupted');
 await page.getByRole('textbox',{name:'Channel message',exact:true}).fill('@Rafa ask @Justine to check this');await page.getByRole('button',{name:'Send channel message',exact:true}).click();await expect(members.getByRole('status')).toContainText('0 of 4');await expect(members.getByRole('status')).not.toContainText('Paused');
 await toggle.uncheck();await expect(limit).toHaveCount(0);expect(await page.evaluate(()=>window.__MONITTER_QA__.calls.filter(c=>c.method==='sendChannelMessage').length)).toBe(1);
 console.log('Agent conversation settings, reply budget display, Stop and new-user round controls passed.');
}finally{await browser.close();}
