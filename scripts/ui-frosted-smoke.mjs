import {chromium,expect} from '@playwright/test';
import {mkdirSync,writeFileSync} from 'node:fs';
mkdirSync('verification',{recursive:true});
const browser=await chromium.launch({headless:true}),page=await browser.newPage({viewport:{width:1440,height:900}}),passed=[],errors=[];
page.on('pageerror',e=>errors.push(e.message));
try{
 await page.addInitScript({path:'scripts/ui-fixture.js'});await page.goto(process.env.MONITTER_TEST_URL||'http://127.0.0.1:18428');await expect(page.getByRole('button',{name:'Monitter menu',exact:true})).toBeVisible();
 await page.evaluate(()=>{const q=window.__MONITTER_QA__,s=q.snapshot(),now=Date.now();s.settings.theme='dark';s.tasks=[{id:'glass',agentId:'atlas',title:'Glass title',status:'completed',nativeSessionId:null,archived:false,createdAt:now,updatedAt:now,hostId:'local',cwd:'/tmp',provider:'codex',model:'',sandbox:'read-only'}];s.messages=Array.from({length:35},(_,i)=>({id:`m${i}`,taskId:'glass',role:i%2?'assistant':'user',text:`Message ${i}. `+'Scrolling content behind the translucent title. '.repeat(9),createdAt:now+i}));q.setSnapshot(s)});
 await page.locator('.sidebar .task-select').filter({hasText:'Glass title'}).click();
 const main=page.locator('.pane-leaf[data-pane-id="main"]'),head=main.locator('.conversation-head'),messages=main.getByRole('region',{name:'Messages'});
 await expect(main.locator('.message-header')).toBeVisible();
 const visual=await head.evaluate(el=>({bg:getComputedStyle(el).backgroundColor,blur:getComputedStyle(el).backdropFilter,opacity:getComputedStyle(el).opacity}));expect(visual.blur).toBe('blur(14px)');expect(visual.opacity).toBe('1');expect(visual.bg).toMatch(/0\.8|\/ 0\.8/);
 await messages.evaluate(el=>el.scrollTop=0);const initial=await head.boundingBox();await messages.evaluate(el=>el.scrollTop=100);await expect.poll(()=>head.boundingBox().then(b=>Math.abs(b.y-initial.y))).toBeLessThan(1);
 const article=await main.locator('.message-content article').first().boundingBox();expect(article.y).toBeLessThan(initial.y+initial.height);expect(article.y+article.height).toBeGreaterThan(initial.y);
 await expect.poll(()=>page.evaluate(()=>document.documentElement.scrollHeight-window.innerHeight)).toBeLessThanOrEqual(1);
 passed.push('title stays pinned above scrolling messages with an 80% background and 14px backdrop blur; text remains opaque and the document stays fixed');
 await main.getByRole('button',{name:'Pane layout'}).click();await page.getByRole('menuitem',{name:'Two columns',exact:true}).click();const other=page.locator('.pane-leaf').last();
 await expect.poll(()=>other.evaluate(el=>getComputedStyle(el).filter)).toBe('grayscale(1)');await expect.poll(()=>main.evaluate(el=>getComputedStyle(el).filter)).toBe('none');
 await other.getByRole('button',{name:'Overview',exact:true}).click();await expect.poll(()=>other.evaluate(el=>getComputedStyle(el).filter)).toBe('none');await expect.poll(()=>main.evaluate(el=>getComputedStyle(el).filter)).toBe('grayscale(1)');
 await page.evaluate(()=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.settings.dimInactivePanes=false;q.setSnapshot(s)});await expect.poll(()=>main.evaluate(el=>getComputedStyle(el).filter)).toBe('none');
 passed.push('inactive panes desaturate, focus restores colour, and disabling dimming disables desaturation');
 expect(errors).toEqual([]);await page.screenshot({path:'verification/ui-frosted.png'});writeFileSync('verification/ui-frosted-results.json',JSON.stringify({passed,errors},null,2));console.log(JSON.stringify({passed,errors},null,2));
}catch(error){await page.screenshot({path:'verification/ui-frosted-failure.png'});writeFileSync('verification/ui-frosted-results.json',JSON.stringify({passed,errors,error:String(error)},null,2));throw error}finally{await browser.close()}
