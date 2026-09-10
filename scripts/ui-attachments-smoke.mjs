import {chromium,expect} from '@playwright/test';
import {mkdirSync,writeFileSync} from 'node:fs';
mkdirSync('verification',{recursive:true});
const browser=await chromium.launch({headless:true}),passed=[],errors=[];
const page=await browser.newPage({viewport:{width:1440,height:900}});
page.on('pageerror',e=>errors.push(e.message));
const calls=method=>page.evaluate(method=>window.__MONITTER_QA__.calls.filter(c=>c.method===method),method);
const main=page.locator('.pane-leaf[data-pane-id="main"]');
const input=main.getByLabel('Task message',{exact:true});
const cards=main.locator('.composer .attachment');
async function newChat(){await page.keyboard.press('Meta+p');await page.getByRole('dialog').getByText('New chat',{exact:true}).click();}
async function seed(fn){await page.evaluate(fn);}
try {
  await page.addInitScript({path:'scripts/ui-fixture.js'});await page.goto(process.env.MONITTER_TEST_URL||'http://127.0.0.1:18421');
  await expect(page.getByRole('button',{name:'Monitter menu',exact:true})).toBeVisible();
  await newChat();
  await input.evaluate(node=>{
    const canvas=document.createElement('canvas');canvas.width=32;canvas.height=24;canvas.getContext('2d').fillRect(0,0,32,24);
    const base64=canvas.toDataURL('image/png').split(',')[1],data=new DataTransfer();
    data.items.add(new File([Uint8Array.from(atob(base64),c=>c.charCodeAt(0))],'pasted.png',{type:'image/png'}));
    node.dispatchEvent(new ClipboardEvent('paste',{clipboardData:data,bubbles:true,cancelable:true}));
  });
  await expect(cards).toHaveCount(1);await expect(cards.getByAltText('Preview of pasted.png')).toBeVisible();
  expect((await calls('createTask')).length).toBe(0);
  await main.locator('.composer').evaluate(node=>{const data=new DataTransfer();data.items.add(new File(['Notes for the agent'],'notes.txt',{type:'text/plain'}));node.dispatchEvent(new DragEvent('drop',{dataTransfer:data,bubbles:true,cancelable:true}));});
  await expect(cards).toHaveCount(2);await expect(cards.filter({hasText:'notes.txt'}).locator('.file-icon')).toBeVisible();
  expect((await calls('storeAttachment'))[0].args.target).toEqual({agentId:'atlas',projectId:null});
  passed.push('image paste previews and file drop icons upload without creating a task');
  await main.getByRole('button',{name:'Send task message',exact:true}).click();
  await expect.poll(()=>calls('sendMessage').then(c=>c.length)).toBe(1);
  const sent=(await calls('sendMessage'))[0].args;expect(sent.text).toBe('');expect(sent.attachmentIds).toHaveLength(2);
  await expect(main.locator('.message .attachment')).toHaveCount(2);await expect(cards).toHaveCount(0);
  expect(await page.evaluate(()=>window.__MONITTER_QA__.snapshot().messages.at(-1).attachments.every(a=>a.path.includes('/.monitter/attachments/')))).toBe(true);
  passed.push('attachment-only send passes registered references and persists image/file cards');
  await newChat();await input.fill('Keep this unsent draft');
  await seed(()=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.attachmentFailure='Disk fixture is full';q.setSnapshot(s)});
  await main.locator('input[type=file]').setInputFiles({name:'failed.txt',mimeType:'text/plain',buffer:Buffer.from('test')});
  await expect(page.getByText('Disk fixture is full',{exact:true})).toBeVisible();await expect(input).toHaveValue('Keep this unsent draft');await expect(cards).toHaveCount(0);
  await seed(()=>{const q=window.__MONITTER_QA__,s=q.snapshot();delete s.attachmentFailure;q.setSnapshot(s)});
  await main.locator('input[type=file]').setInputFiles({name:'kept.txt',mimeType:'text/plain',buffer:Buffer.from('test')});await expect(cards).toHaveCount(1);
  passed.push('file picker failures remain visible and preserve unsent text; retry uploads');
  await main.getByRole('button',{name:'Pane layout',exact:true}).click();await page.getByRole('menuitem',{name:'Two columns',exact:true}).click();
  const second=page.locator('.pane-leaf').last(),tab=main.locator('.tabs .tab-entry.active > .tab');
  const data=await page.evaluateHandle(()=>new DataTransfer());await tab.dispatchEvent('dragstart',{dataTransfer:data});const box=await second.boundingBox();
  const event={dataTransfer:data,clientX:box.x+box.width/2,clientY:box.y+box.height/2};await second.dispatchEvent('dragover',event);await second.dispatchEvent('drop',event);await data.dispose();
  await expect(second.getByLabel('Task message',{exact:true})).toHaveValue('Keep this unsent draft');await expect(second.locator('.composer .attachment')).toHaveCount(1);
  await second.getByRole('button',{name:'Send task message',exact:true}).click();await expect.poll(()=>calls('sendMessage').then(c=>c.length)).toBe(2);expect((await calls('sendMessage'))[1].args.attachmentIds).toHaveLength(1);
  passed.push('moving a draft between panes preserves uploaded references and message text');
  expect(errors).toEqual([]);await page.screenshot({path:'verification/ui-attachments.png'});
  console.log(JSON.stringify({passed,errors},null,2));writeFileSync('verification/ui-attachments-results.json',JSON.stringify({passed,errors},null,2));
} catch(error){writeFileSync('verification/ui-attachments-results.json',JSON.stringify({passed,errors,error:String(error)},null,2));await page.screenshot({path:'verification/ui-attachments-failure.png'});throw error}finally{await browser.close()}
