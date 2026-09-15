import { chromium, expect } from '@playwright/test';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18464';
const browser = await chromium.launch({ headless: true });

try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const errors = [];
  page.on('pageerror', error=>errors.push(error.message));
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.addInitScript(() => {
    const qa=window.__MONITTER_QA__, state=qa.snapshot();
    state.hosts.push({ ...state.hosts[0], id:'remote', name:'Mira', kind:'ssh', address:'mira.local' });
    state.agents[0]={ ...state.agents[0], model:'gpt-test' };
    state.settings.compressToolCalls=true;
    state.agents.push({ ...state.agents[0], id:'remote-agent', name:'Remote agent', hostId:'remote', provider:'claude', model:'sonnet' });
    state.tasks=[{ id:'reasoning-task',agentId:'atlas',projectId:null,title:'Reasoning run',nativeSessionId:null,status:'completed',archived:false,createdAt:1,updatedAt:20,parentTaskId:null,channelId:null,hostId:'local',cwd:'/tmp',provider:'codex',model:'gpt-test',sandbox:'read-only' }];
    state.messages=[{ id:'user',taskId:'reasoning-task',role:'user',text:'Investigate this.',createdAt:2,attachments:[] },{ id:'assistant',taskId:'reasoning-task',role:'assistant',text:'Done.',createdAt:10,attachments:[] }];
    state.events=[
      {id:'r1',taskId:'reasoning-task',kind:'reasoning',title:'Reasoning',detail:'First check.',createdAt:4},
      {id:'r2',taskId:'reasoning-task',kind:'reasoning',title:'Reasoning',detail:'Second check.',createdAt:5},
      {id:'r3',taskId:'reasoning-task',kind:'reasoning',title:'Reasoning',detail:'Third check.',createdAt:6},
      {id:'web1',taskId:'reasoning-task',kind:'tool',title:'Web search',detail:'{}',createdAt:12},
      {id:'web2',taskId:'reasoning-task',kind:'tool',title:'Web search',detail:'{}',createdAt:13},
      {id:'pending1',taskId:'reasoning-task',kind:'reasoning',title:'Reasoning',detail:'',createdAt:14},
      {id:'web3',taskId:'reasoning-task',kind:'tool',title:'Web search',detail:'{}',createdAt:15},
      {id:'web4',taskId:'reasoning-task',kind:'tool',title:'Web search',detail:'{}',createdAt:16},
      {id:'compact1',taskId:'reasoning-task',kind:'tool',title:'ContextCompaction',detail:'{"type":"ContextCompaction","id":"compact","monitterPhase":"started"}',createdAt:17},
      {id:'compact2',taskId:'reasoning-task',kind:'tool',title:'ContextCompaction',detail:'{"type":"ContextCompaction","id":"compact","monitterPhase":"completed"}',createdAt:18},
      {id:'pending2',taskId:'reasoning-task',kind:'reasoning',title:'Reasoning',detail:'',createdAt:19},
      {id:'web5',taskId:'reasoning-task',kind:'tool',title:'Web search',detail:'{}',createdAt:20},
      {id:'web6',taskId:'reasoning-task',kind:'tool',title:'Web search',detail:'{}',createdAt:21},
    ];
    qa.setSnapshot(state);
    localStorage.setItem('monitter.sidebar-view.v2:web','standard');
    sessionStorage.setItem('monitter.sidebar-view.v2:web','standard');
    localStorage.removeItem('monitter.appearance.terminal-theme.v1');
    localStorage.removeItem('monitter.appearance.app-theme.v1');
    localStorage.removeItem('monitter.appearance.app-theme-pair.v2');
    localStorage.removeItem('monitter.appearance.browser-colours.v1');
    localStorage.removeItem('monitter.appearance.mode.v1');
    localStorage.removeItem('monitter.workspaces.v2');
  });
  await page.goto(url, { waitUntil:'domcontentloaded', timeout:60000 });

  await expect(page.locator('.workspace > .topbar')).toHaveCount(0);
  await expect(page.getByRole('button',{name:'Open terminal'})).toHaveCount(0);
  await page.getByRole('button',{name:'Standard view',exact:true}).click();
  await expect(page.locator('.agent-location[data-host-kind="local"]')).toHaveAttribute('aria-label','Local host: This Mac');
  await expect(page.locator('.agent-location[data-host-kind="ssh"]')).toHaveAttribute('aria-label','Remote host: Mira');

  await page.locator('[data-task-id="reasoning-task"] .task-select').click();
  await expect(page.getByLabel('Reasoning summary',{exact:true})).toHaveCount(1);
  await page.getByLabel('Reasoning summary',{exact:true}).click();
  await expect(page.locator('details.reasoning .activity-body')).toContainText('First check.');
  await expect(page.locator('details.reasoning .activity-body')).toContainText('Third check.');
  await expect(page.getByText('Searched the web · 6 tool calls',{exact:true})).toBeVisible();
  await expect(page.locator('.reasoning-pending')).toHaveCount(0);

  await page.keyboard.press('Meta+,');
  await expect(page.getByText('Light theme',{exact:true})).toBeVisible();
  await expect(page.getByText('Dark theme',{exact:true})).toBeVisible();
  await expect.poll(()=>page.evaluate(()=>getComputedStyle(document.documentElement).getPropertyValue('--accent').trim())).toBe('#00A8F0');
  await expect(page.locator('meta[name="theme-color"]')).toHaveAttribute('content','#EAEEF0');
  await expect.poll(()=>page.evaluate(()=>({html:document.documentElement.style.backgroundColor,body:document.body.style.backgroundColor,colours:JSON.parse(localStorage.getItem('monitter.appearance.browser-colours.v1')||'null'),mode:localStorage.getItem('monitter.appearance.mode.v1')}))).toEqual({html:'rgb(234, 238, 240)',body:'rgb(234, 238, 240)',colours:{light:'#EAEEF0',dark:'#0E171A'},mode:'light'});
  await page.locator('summary[aria-label="Dark theme: Monitter"]').click();
  const darkChoices=page.getByRole('listbox',{name:'Dark theme choices',exact:true});
  await expect(darkChoices.getByRole('option')).toHaveCount(10);
  await darkChoices.getByRole('option',{name:'Nord',exact:true}).click();
  await expect.poll(()=>page.evaluate(()=>({stored:JSON.parse(localStorage.getItem('monitter.appearance.app-theme-pair.v2')||'null'),light:document.documentElement.dataset.appThemeLight,dark:document.documentElement.dataset.appThemeDark,mode:document.documentElement.dataset.theme,paper:getComputedStyle(document.documentElement).getPropertyValue('--paper-base').trim()}))).toEqual({stored:{light:'monitter',dark:'nord',accent:null,contrast:0},light:'monitter',dark:'nord',mode:'light',paper:'#F7F7F7'});
  await expect(page.getByRole('region',{name:'Appearance mode',exact:true})).toBeVisible();
  await page.getByRole('button',{name:'dark',exact:true}).click();
  await expect.poll(()=>page.evaluate(()=>({mode:document.documentElement.dataset.theme,paper:getComputedStyle(document.documentElement).getPropertyValue('--paper-base').trim(),accent:getComputedStyle(document.documentElement).getPropertyValue('--accent').trim()}))).toEqual({mode:'dark',paper:'#2E3440',accent:'#88C0D0'});
  await page.getByText('Advanced theme controls',{exact:true}).click();
  await page.getByRole('button',{name:'#c44c79',exact:true}).click();
  await expect.poll(()=>page.evaluate(()=>getComputedStyle(document.documentElement).getPropertyValue('--accent').trim())).toBe('#c44c79');
  await page.getByRole('button',{name:'light',exact:true}).click();
  await page.locator('summary[aria-label="Light theme: Monitter"]').click();
  await page.getByRole('listbox',{name:'Light theme choices',exact:true}).getByRole('option',{name:'Catppuccin',exact:true}).click();
  await expect.poll(()=>page.evaluate(()=>({paper:getComputedStyle(document.documentElement).getPropertyValue('--paper-base').trim(),accent:getComputedStyle(document.documentElement).getPropertyValue('--accent').trim()}))).toEqual({paper:'#EFF1F5',accent:'#c44c79'});
  await page.getByRole('button',{name:'Use theme accents',exact:true}).click();
  await expect.poll(()=>page.evaluate(()=>getComputedStyle(document.documentElement).getPropertyValue('--accent').trim())).toBe('#1E66F5');
  const picker=page.getByRole('combobox',{name:'Terminal colour theme',exact:true});
  await expect(picker.locator('option')).toHaveCount(10);
  await picker.selectOption('catppuccin-mocha');
  await expect.poll(()=>page.evaluate(()=>({stored:localStorage.getItem('monitter.appearance.terminal-theme.v1'),theme:document.documentElement.dataset.terminalTheme,background:getComputedStyle(document.documentElement).getPropertyValue('--terminal-background').trim()}))).toEqual({stored:'catppuccin-mocha',theme:'catppuccin-mocha',background:'#1E1E2E'});
  expect(errors).toEqual([]);
  console.log('Chrome-free overview, host icons, consolidated reasoning, and whole-app/terminal theme selection passed.');
} finally {
  await browser.close();
}
