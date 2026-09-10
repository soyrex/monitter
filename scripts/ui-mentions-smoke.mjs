import {chromium,expect} from '@playwright/test';
const browser=await chromium.launch({headless:true});
try {
 const page=await browser.newPage({viewport:{width:1440,height:900}});
 await page.addInitScript({path:'scripts/ui-fixture.js'});await page.goto('http://127.0.0.1:18433');
 await expect(page.getByRole('button',{name:'Monitter menu',exact:true})).toBeVisible({timeout:60000});
 await page.evaluate(()=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.agents=[{...s.agents[0],id:'rafa',name:'Rafa'},{...s.agents[0],id:'justine',name:'Justine'}];s.channels=[{id:'everyone',name:'Everyone',description:'',agentIds:['rafa','justine'],messages:[]}];q.setSnapshot(s);});
 await page.getByRole('complementary',{name:'Agents and tasks'}).getByRole('button',{name:/Everyone/}).click();
 const input=page.getByRole('textbox',{name:'Channel message',exact:true});
 await input.pressSequentially('@rafa hello');
 await expect(page.locator('.mention-pill')).toHaveText('@rafa');await expect(page.getByRole('button',{name:'Rafa',exact:true})).toHaveAttribute('aria-pressed','true');
 await input.fill('@justine hello');await expect(page.locator('.mention-pill')).toHaveText('@justine');await expect(page.getByRole('button',{name:'Rafa',exact:true})).toHaveAttribute('aria-pressed','false');
 await page.getByRole('button',{name:'Rafa',exact:true}).click();await input.fill('@justine\nhello @unknown');
 await page.getByRole('button',{name:'Send channel message',exact:true}).click();
 const sent=await page.evaluate(()=>window.__MONITTER_QA__.calls.filter(c=>c.method==='sendChannelMessage').at(-1));
 expect(sent.args.agentIds.sort()).toEqual(["justine","rafa"]);
 expect(sent.args.text).toBe("@justine\nhello @unknown");
 await expect(input).toHaveValue('');
 console.log('Mention pills, recipient selection/removal, multiline sending passed.');
}finally{await browser.close();}
