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
      state.agents[0].provider = 'acp';
      state.agents[0].model = 'gemini-3.5-flash';
      state.agents[0].acp = {command:'/test/local/gemini',args:['--acp']};
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
          {id:'agy-headless',name:'AGY CLI (Monitter bridge)',description:'Local AGY headless chats; collaboration unavailable',integration:'bridge',sourceUrl:'https://antigravity.google/docs/cli/headless/',detected:hostId==='local',launch:{command:'monitter-agy-acp',args:[]}},
          {id:'antigravity-acp',name:'Google Antigravity',description:'Official AGY ACP server',integration:'native',sourceUrl:'https://github.com/agentclientprotocol/registry/blob/main/antigravity-acp/agent.json',detected:true,launch:{command:hostId==='local'?'/test/local/agy_acp_server.par':'/test/remote/agy_acp_server.par',args:[]}},
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
    await expect(providerCard.getByRole('heading',{name:'Gemini',exact:true})).toBeVisible();
    await activate(providerCard.getByRole('button',{name:'Change',exact:true}));
    await expect(providerCard.getByRole('button').filter({hasText:'Mona'}).first()).toBeVisible();
    await activate(providerCard.getByRole('button').filter({hasText:'Mona'}).first());
    await expect(providerCard.getByRole('heading',{name:'Mona',exact:true})).toBeVisible();
    const picker = settings.getByRole('region',{name:'Launch behaviour'});
    await expect(picker.getByLabel('Executable',{exact:true})).toHaveValue('mona-acp');
    await activate(providerCard.getByRole('button',{name:'Change',exact:true}));
    await activate(settings.getByRole('region',{name:'Choose a provider'}).getByRole('button',{name:/Gemini/}));
    await expect(picker.getByLabel('Executable',{exact:true})).toHaveValue('monitter-agy-acp');
    await expect(picker.getByLabel('Argument 1',{exact:true})).toHaveCount(0);
    await activate(settings.locator('details.agent-profile').filter({hasText:'Collaboration profile'}).locator('summary'));
    await expect(settings.getByText('AGY CLI chats cannot receive Monitter collaboration tools.',{exact:false})).toBeVisible();
    await expect(settings.getByRole('switch',{name:'Available for collaboration'})).toBeDisabled();
    await expect(picker.getByRole('button',{name:/AGY CLI \(Monitter bridge\) Detected/})).toBeVisible();
    await activate(picker.getByRole('button',{name:/AGY CLI \(Monitter bridge\) Detected/}));
    await expect(picker.getByLabel('Executable',{exact:true})).toHaveValue('monitter-agy-acp');
    await expect(picker.getByRole('button',{name:/Google Antigravity Detected/})).toBeVisible();
    await activate(picker.getByRole('button',{name:/Google Antigravity Detected/}));
    await expect(picker.getByLabel('Executable',{exact:true})).toHaveValue('/test/local/agy_acp_server.par');
    await expect(picker.getByLabel('Argument 1',{exact:true})).toHaveCount(0);
    expect(await page.evaluate(() => window.__ACP_VERIFY_CALLS__)).toEqual([]);
    await activate(picker.getByRole('button',{name:'Verify connection',exact:true}));
    await expect(picker.getByText('v1 verified',{exact:false})).toBeVisible();
    await expect(picker.getByText('Per-turn Jev routing',{exact:true})).toBeVisible();
    await expect(picker.getByText('Auth loader',{exact:true})).toBeVisible();
    await expect(picker.getByText('Reasoning effort',{exact:true})).toBeVisible();
    expect(await page.evaluate(() => window.__ACP_VERIFY_CALLS__)).toEqual([{hostId:'local',launch:{command:'/test/local/agy_acp_server.par',args:[]}}]);
    await picker.getByLabel('Search presets').fill('Pi');
    await expect(picker.getByRole('button',{name:/Google Antigravity Not found|Google Antigravity Detected/})).toHaveCount(0);
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
    await expect(picker.getByRole('button',{name:/Google Antigravity Detected/})).toContainText('/test/remote/agy_acp_server.par');
    await activate(picker.getByRole('button',{name:/Google Antigravity Detected/}));
    await expect(picker.getByLabel('Executable',{exact:true})).toHaveValue('/test/remote/agy_acp_server.par');
    expect(await page.evaluate(() => window.__ACP_DISCOVERY_CALLS__)).toContain('remote-acp');
    expect(errors.filter(message=>!message.includes('transformCallback'))).toEqual([]);
    await mkdir('verification/acp',{recursive:true});
    await picker.screenshot({path:`verification/acp/setup-${engine.name()}.png`});
    console.log(`${engine.name()}: ACP catalog, exact argv, host discovery, explicit verification and stale-result invalidation passed`);
  } finally { await browser.close(); }
}
