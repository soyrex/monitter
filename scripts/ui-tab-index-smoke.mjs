import {chromium,expect} from '@playwright/test';
import {readFileSync} from 'node:fs';
const browser=await chromium.launch({headless:true});
try{
 for(const shortcutMode of ['standard','vim']){
 const page=await browser.newPage({viewport:{width:1500,height:950}});
 await page.addInitScript({content:readFileSync('scripts/ui-fixture.js','utf8')+`Object.defineProperty(navigator,'platform',{value:'MacIntel'});const q=window.__MONITTER_QA__,s=q.snapshot(),n=Date.now();s.settings.shortcutMode='${shortcutMode}';s.tasks=['One','Two'].map((title,i)=>({id:'index-'+i,agentId:'atlas',title,nativeSessionId:null,status:'completed',archived:false,createdAt:n,updatedAt:n,parentTaskId:null,channelId:null,projectId:null,hostId:'local',cwd:'/tmp',provider:'codex',model:'',sandbox:'read-only'}));q.setSnapshot(s);`});
 await page.goto('http://127.0.0.1:18433');await page.locator('.sidebar .task-select').filter({hasText:'One'}).click({timeout:60000});await page.locator('.sidebar .task-select').filter({hasText:'Two'}).click();
 await page.keyboard.press('Meta+1');await expect(page.locator('.dashboard-tab')).toHaveClass(/active/);await page.keyboard.press('Meta+2');await expect(page.locator('[data-tab-id="index-0"]')).toHaveClass(/active/);
 async function command(value){await page.keyboard.press('Meta+p');await page.getByText('Vim command',{exact:true}).click();const input=page.getByRole('textbox',{name:'Monitter command'});await input.fill(value);await input.press('Enter');}
 await command('vsplit');await command('tabnew');await expect(page.locator('.pane-leaf')).toHaveCount(2);const child=page.locator('.pane-leaf').last();
 await page.keyboard.down('Meta');await expect(page.locator('.show-tab-index')).toHaveCount(1);await expect(child.locator('.show-tab-index')).toBeVisible();expect(await child.locator('.tab-entry').first().evaluate(el=>getComputedStyle(el,'::after').content)).toContain('counter');await page.keyboard.press('1');await expect(child.locator('.tab-entry.active')).toHaveCount(1);await page.keyboard.up('Meta');await expect(page.locator('.show-tab-index')).toHaveCount(0);
 await expect(page.locator('.pane-leaf').first().locator('[data-tab-id="index-0"]')).toHaveClass(/active/);await page.close();
 }
 console.log('Cmd tab indexes include dashboard, select within focusedpane in both modes, and appear only while held.');
}finally{await browser.close();}
