import { chromium, webkit, expect } from '@playwright/test';

const url=process.env.MONITTER_TEST_URL||'http://127.0.0.1:18464';
const longTitle='An unusually long recent chat title that should determine the fitted sidebar width';
const task={id:'wide-chat',agentId:'atlas',title:longTitle,nativeSessionId:null,status:'idle',archived:false,createdAt:1,updatedAt:1,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp',provider:'codex',model:'',sandbox:'read-only'};
for(const engine of [chromium,webkit]){
  const browser=await engine.launch({headless:true});
  try{
    const page=await browser.newPage({viewport:{width:1600,height:900}});
    await page.addInitScript({path:'scripts/ui-fixture.js'});
    await page.addInitScript(({task})=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.settings.interfaceScale=125;s.tasks=[task];q.setSnapshot(s);},{task});
    await page.goto(url,{waitUntil:'domcontentloaded',timeout:240_000});
    const sidebar=page.locator('.app-shell:not(.embedded) > .sidebar'),separator=page.getByRole('separator',{name:'Resize main sidebar'});
    await expect(separator).toBeVisible({timeout:240_000});
    const before=await sidebar.boundingBox();await separator.dblclick();
    await expect.poll(async()=>Math.round((await sidebar.boundingBox()).width)).toBeGreaterThan(Math.round(before.width));
    const visible=await page.locator('.sidebar .chat-copy > span').filter({hasText:longTitle}).evaluate((node)=>{
      const range=document.createRange();range.selectNodeContents(node);const text=range.getBoundingClientRect(),side=node.closest('.sidebar').getBoundingClientRect();return text.right<=side.right-20;
    });
    expect(visible).toBe(true);
    console.log(`${engine.name()}: double-click fits the main sidebar to its content`);
  }finally{await browser.close();}
}
