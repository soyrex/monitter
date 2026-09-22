import { chromium, webkit, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { mkdir } from 'node:fs/promises';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18458';
const engines = process.env.MONITTER_TEST_BROWSER === 'webkit'
  ? [[webkit,{width:407,height:900}]]
  : [[chromium,{width:1440,height:1000}],[webkit,{width:407,height:900}]];
for (const [engine, viewport] of engines) {
  const browser = await engine.launch({headless:true});
  try {
    const page = await browser.newPage({viewport,hasTouch:engine===webkit});
    const activate = locator => engine === webkit ? locator.tap() : locator.click();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript({content:readFileSync('scripts/ui-fixture.js','utf8')});
    await page.addInitScript(() => {
      const state = window.__MONITTER_QA__.snapshot();
      state.hosts.push({...state.hosts[0],id:'remote-acp',name:'Remote test host',kind:'ssh',address:'fixture.invalid'});
      window.__MONITTER_QA__.setSnapshot(state);
      window.__ACP_DISCOVERY_CALLS__ = [];
      window.__ACP_VERIFY_CALLS__ = [];
      window.__MONITTER_BRIDGE__.verifyAcpAgent = async (hostId, launch) => {
        window.__ACP_VERIFY_CALLS__.push({hostId,launch});
        return {protocolVersion:1,agentName:'Fixture agent',agentVersion:'1.0',loadSession:true,resumeSession:false,image:false,audio:false,embeddedContext:false,jevRouting:true,authLoader:true,reasoningEffort:true};
      };
      window.__MONITTER_BRIDGE__.discoverAcpAgents = async hostId => {
        window.__ACP_DISCOVERY_CALLS__.push(hostId);
        return [
          {id:'gemini',name:'Gemini CLI',description:'Native ACP integration',integration:'native',sourceUrl:'https://geminicli.com',detected:true,launch:{command:hostId==='local'?'/test/local/gemini':'/test/remote/gemini',args:['--acp']}},
          {id:'pi',name:'Pi (ACP bridge)',description:'Requires pi-acp; Pi native RPC is not ACP',integration:'bridge',sourceUrl:'https://github.com/victor-software-house/pi-acp',detected:false,launch:{command:'pi-acp',args:[]}},
        ];
      };
    });
    await page.goto(url,{waitUntil:'domcontentloaded',timeout:120000});
    await expect(page.locator('.sidebar')).toBeVisible({timeout:60000});
    await page.keyboard.press('Meta+,');
    const settings = page.locator('.settings-pane');
    await activate(settings.getByRole('button',{name:'Agents',exact:true}));
    await activate(settings.getByRole('button',{name:'Edit Atlas',exact:true}));
    const providerCard = settings.locator('.agent-provider-card');
    await activate(providerCard.getByRole('button',{name:'Change',exact:true}));
    await expect(providerCard.getByRole('button').filter({hasText:'Mona'}).first()).toBeVisible();
    await activate(providerCard.getByRole('button').filter({hasText:'Mona'}).first());
    await expect(providerCard.getByRole('heading',{name:'Mona',exact:true})).toBeVisible();
    const picker = settings.getByRole('region',{name:'Launch behaviour'});
    await expect(picker.getByLabel('Executable',{exact:true})).toHaveValue('mona-acp');
    await expect(picker.getByRole('button',{name:/Gemini CLI Detected/})).toBeVisible();
    await activate(picker.getByRole('button',{name:/Gemini CLI Detected/}));
    await expect(picker.getByLabel('Executable',{exact:true})).toHaveValue('/test/local/gemini');
    await expect(picker.getByLabel('Argument 1',{exact:true})).toHaveValue('--acp');
    expect(await page.evaluate(() => window.__ACP_VERIFY_CALLS__)).toEqual([]);
    await activate(picker.getByRole('button',{name:'Verify connection',exact:true}));
    await expect(picker.getByText('v1 verified',{exact:false})).toBeVisible();
    await expect(picker.getByText('Per-turn Jev routing',{exact:true})).toBeVisible();
    await expect(picker.getByText('Auth loader',{exact:true})).toBeVisible();
    await expect(picker.getByText('Reasoning effort',{exact:true})).toBeVisible();
    expect(await page.evaluate(() => window.__ACP_VERIFY_CALLS__)).toEqual([{hostId:'local',launch:{command:'/test/local/gemini',args:['--acp']}}]);
    await picker.getByLabel('Search presets').fill('Pi');
    await expect(picker.getByRole('button',{name:/Gemini CLI Not found|Gemini CLI Detected/})).toHaveCount(0);
    await expect(picker.getByRole('button',{name:/Pi \(ACP bridge\) Not found/})).toBeVisible();
    await activate(picker.getByRole('button',{name:/Custom executable/}));
    await expect(picker.getByText('v1 verified',{exact:false})).toHaveCount(0);
    await picker.getByLabel('Executable',{exact:true}).fill('/path with spaces/custom-agent');
    await activate(picker.getByRole('button',{name:'Add argument'}));
    await picker.getByLabel('Argument 1',{exact:true}).fill('literal $(value) with spaces');
    await activate(settings.getByRole('button',{name:'Save agent',exact:true}));
    await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().agents[0].acp)).toEqual({command:'/path with spaces/custom-agent',args:['literal $(value) with spaces']});
    await activate(picker.getByRole('button',{name:'Verify connection',exact:true}));
    await expect(picker.getByText('v1 verified',{exact:false})).toBeVisible();
    await expect(picker.getByText('Per-turn Jev routing',{exact:true})).toBeVisible();
    await expect(picker.getByText('Auth loader',{exact:true})).toBeVisible();
    await expect(picker.getByText('Reasoning effort',{exact:true})).toBeVisible();
    await settings.getByRole('combobox',{name:'Host',exact:true}).selectOption('remote-acp');
    await expect(picker.getByText('v1 verified',{exact:false})).toHaveCount(0);
    await picker.getByLabel('Search presets').fill('');
    await expect(picker.getByRole('button',{name:/Gemini CLI Detected/})).toContainText('/test/remote/gemini');
    await activate(picker.getByRole('button',{name:/Gemini CLI Detected/}));
    await expect(picker.getByLabel('Executable',{exact:true})).toHaveValue('/test/remote/gemini');
    expect(await page.evaluate(() => window.__ACP_DISCOVERY_CALLS__)).toContain('remote-acp');
    expect(errors.filter(message=>!message.includes('transformCallback'))).toEqual([]);
    await mkdir('verification/acp',{recursive:true});
    await picker.screenshot({path:`verification/acp/setup-${engine.name()}.png`});
    console.log(`${engine.name()}: ACP catalog, exact argv, host discovery, explicit verification and stale-result invalidation passed`);
  } finally { await browser.close(); }
}
