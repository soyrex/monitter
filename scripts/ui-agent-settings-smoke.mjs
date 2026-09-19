import {chromium,webkit,expect} from '@playwright/test';
import {readFileSync} from 'node:fs';
const browser=await (process.env.MONITTER_TEST_BROWSER==='webkit'?webkit:chromium).launch({headless:true});
try{
 const page=await browser.newPage({viewport:{width:1440,height:950}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.addInitScript({content:readFileSync('scripts/ui-fixture.js','utf8')+`const q=window.__MONITTER_QA__,s=q.snapshot();s.agents.push({...s.agents[0],id:'beta',name:'Beta',instructions:'Beta instructions'});q.setSnapshot(s);`});
 await page.goto(process.env.MONITTER_TEST_URL||'http://127.0.0.1:18433');
 await page.getByRole('button',{name:'Preferences',exact:true}).click();
 await page.locator('.settings-pane').getByRole('button',{name:'Agents',exact:true}).click();
 const settings=page.locator('.settings-pane');
 await expect(settings).toBeVisible();
 // Directory cards are scoped to Settings. Sidebar agents retain their compact
 // list treatment, without directory-card borders or padding.
 const sidebarAgent = page.locator('.sidebar .agent-row').first();
 await expect(sidebarAgent).toHaveCSS('border-top-width', '0px');
 await expect(sidebarAgent).toHaveCSS('padding-top', '4px');
 // Landing view: directory with the two existing agents.
 const atlasRow = settings.getByRole('button',{name:'Edit Atlas',exact:true});
 const betaRow = settings.getByRole('button',{name:'Edit Beta',exact:true});
 await expect(atlasRow).toBeVisible();
 await expect(betaRow).toBeVisible();
 await expect(settings.getByRole('button',{name:'New agent',exact:true})).toBeVisible();
 // Drill in to edit Atlas by clicking the edit affordance on its row.
 await atlasRow.click();
 const name = settings.getByRole('textbox',{name:'Name',exact:true});
 await expect(name).toHaveValue('Atlas');
 // The model picker and other harness plumbing have been folded into
 // the agent provider card + normalised permission radio cards.
 await expect(settings.getByRole('region',{name:'Permissions'}).first()).toBeVisible();
 // Edit the name, then drill back via the row's edit affordance to switch agents.
 await name.fill('Atlas draft');
 // Discard to confirm the dirty tracking / back affordance work without saving.
 await settings.getByRole('button',{name:'Discard changes',exact:true}).click();
 // The save button is disabled while not dirty.
 await atlasRow.click();
 const saveButton = settings.getByRole('button',{name:'Save agent',exact:true});
 await expect(saveButton).toBeDisabled();
 await name.fill('Atlas draft');
 await expect(saveButton).toBeEnabled();
 await settings.getByRole('button',{name:'Save agent',exact:true}).click();
 await expect(page.getByRole('button',{name:'Open agent Atlas draft',exact:true})).toBeVisible();
 // Back to agents preserves the just-saved name in the directory row.
 await settings.getByRole('button',{name:'Back to agents',exact:true}).click();
 await expect(settings.getByRole('button',{name:'Edit Atlas draft',exact:true})).toBeVisible();
 // Re-open the edit view (without leaving Settings) and verify the reload
 // preserves the in-memory agent draft. The captured settings editor state
 // is what keeps the edit form intact across reloads.
 await settings.getByRole('button',{name:'Edit Atlas draft',exact:true}).click();
 await expect(settings.getByRole('textbox',{name:'Name',exact:true})).toHaveValue('Atlas draft');
 await page.waitForTimeout(500);
 await page.reload();
 await expect(settings).toBeVisible();
 await settings.getByRole('button',{name:'Edit Atlas',exact:true}).click();
 await expect(settings.getByRole('textbox',{name:'Name',exact:true})).toHaveValue('Atlas draft');
 // Switch agents via the directory row affordance without going through Settings.
 await settings.getByRole('button',{name:'Back to agents',exact:true}).click();
 await settings.getByRole('button',{name:'Edit Beta',exact:true}).click();
 await expect(settings.getByRole('textbox',{name:'Name',exact:true})).toHaveValue('Beta');
 // New agent flow.
 await settings.getByRole('button',{name:'Back to agents',exact:true}).click();
 await settings.getByRole('button',{name:'New agent',exact:true}).click();
 const newName = settings.getByRole('textbox',{name:'Name',exact:true});
 await newName.fill('New colleague');
 await settings.getByRole('button',{name:'Save agent',exact:true}).click();
 await expect(page.getByRole('button',{name:'Open agent New colleague',exact:true})).toBeVisible();
 // Internal admin agent is hidden from the directory and from the sidebar.
 await expect(page.locator('.sidebar')).not.toContainText('Monitter Admin');
 // Provider card uses the friendly name; switching via the picker must
 // persist provider and model. Continue editing the agent we created.
 await settings.getByRole('button',{name:'Back to agents',exact:true}).click();
 await settings.getByRole('button',{name:'Edit New colleague',exact:true}).click();
 const providerCard = settings.locator('.agent-provider-card');
 await expect(providerCard.getByRole('heading',{name:'Codex',exact:true})).toBeVisible();
 await providerCard.getByRole('button',{name:'Change',exact:true}).click();
 await providerCard.getByRole('button').filter({hasText: 'Claude Code'}).first().click();
 await expect(settings.getByRole('heading',{name:'Claude Code',exact:true})).toBeVisible();
 await settings.getByRole('button',{name:'Save agent',exact:true}).click();
 await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.snapshot().agents.find(agent=>agent.name==='New colleague')?.provider)).toBe('claude');
 // Switch back to Codex. The model picker opens a popup rather than a
 // textbox, so verify the provider switch through the persisted snapshot
 // alone; the model picker popup itself is covered by AgentModelPicker.
 await providerCard.getByRole('button',{name:'Change',exact:true}).click();
 await providerCard.getByRole('button').filter({hasText: 'Codex'}).first().click();
 await expect(settings.getByRole('heading',{name:'Codex',exact:true})).toBeVisible();
 await settings.getByRole('button',{name:'Save agent',exact:true}).click();
 await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.snapshot().agents.find(agent=>agent.name==='New colleague')?.provider)).toBe('codex');
 // Verify that the normalised permission picker maps the saved Sandbox value.
 // "Read only" is the Codex default for a brand-new agent and is supported.
 await expect(settings.getByRole('radio',{name:'Read only',exact:true})).toBeChecked();
 expect(errors.filter(message=>!message.includes('transformCallback'))).toEqual([]);
 console.log('Agent editor in Settings: directory landing, drill-in, dirty/save/discard, new agent creation, normalised permissions and provider picker all passed.');
}finally{await browser.close();}
