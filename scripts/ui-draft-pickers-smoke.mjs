// Isolated Playwright smoke for the new searchable pickers in the New Chat draft.
// Verifies: (1) opening a draft tab focuses the Agent picker; (2) keyboard
// selecting an agent cascades focus to Project; (3) selecting a project cascades
// focus to the composer textarea; (4) the search input narrows the option list.
//
// Run with: node scripts/ui-draft-pickers-smoke.mjs
// Requires the dev server to be running on MONITTER_TEST_URL (default 127.0.0.1:18450).

import { chromium, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1200, height: 800 } });
  const fixture = readFileSync('scripts/ui-fixture.js', 'utf8');
  // Augment the fixture with two extra agents and two projects so both pickers
  // have more than one option, which forces the search input to render.
  const augmentation = `
    state.agents.push(
      {id:'mariner',avatar:null,name:'Mariner',description:'Supply chain ops',instructions:'',provider:'anthropic',model:'',hostId:'local',cwd:local.defaultCwd,color:'#6f4f99',sandbox:'read-only',expertise:[],responsibilities:[],skills:[],collaborationEnabled:true},
      {id:'cobalt',avatar:null,name:'Cobalt',description:'Code reviewer',instructions:'',provider:'codex',model:'',hostId:'local',cwd:local.defaultCwd,color:'#3a4f7d',sandbox:'read-only',expertise:[],responsibilities:[],skills:[],collaborationEnabled:true}
    );
    state.projects.push(
      {id:'seafood',name:'Seafood ops',description:'Holt Seafood workspace',icon:'',color:'#2c8d6a',workspaces:[{hostId:'local',cwd:'/tmp/monitter-ui-test'}]},
      {id:'gbrain',name:'gbrain',description:'Second brain workspace',icon:'',color:'#8c5b3a',workspaces:[{hostId:'local',cwd:'/tmp/monitter-ui-test/gbrain'}]}
    );
  `;
  await page.addInitScript({ content: fixture + augmentation + '; window.__state=state;' });
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18450', { waitUntil: 'domcontentloaded', timeout: 120000 });

  // 1. Open a draft tab. The sidebar "New chat with Atlas" button is what the
  // other new-chat smoke tests already exercise, so reusing it keeps us
  // consistent.
  await page.getByRole('button', { name: 'New chat with Atlas', exact: true }).click({ timeout: 180000 });

  const draft = page.getByRole('region', { name: 'New chat draft', exact: true });
  await expect(draft).toBeVisible();

  // 2. Tab-open focus lands on the Agent picker trigger.
  const agentTrigger = draft.getByRole('button', { name: /Agent:/, exact: false }).first();
  await expect(agentTrigger).toBeFocused({ timeout: 5000 });

  // 3. Open the picker and pick Mariner via keyboard.
  await agentTrigger.press('Enter');
  const agentSearch = draft.getByRole('searchbox', { name: /Search agent/i });
  await expect(agentSearch).toBeVisible();
  await agentSearch.fill('Mari');
  await agentSearch.press('ArrowDown');
  await agentSearch.press('Enter');

  // 4. After Agent pick, focus should cascade to the Project picker.
  const projectTrigger = draft.getByRole('button', { name: /Project:/, exact: false }).first();
  await expect(projectTrigger).toBeFocused({ timeout: 5000 });

  // 5. Pick Seafood ops via keyboard.
  await projectTrigger.press('Enter');
  const projectSearch = draft.getByRole('searchbox', { name: /Search project/i });
  await expect(projectSearch).toBeVisible();
  await projectSearch.fill('Sea');
  await projectSearch.press('ArrowDown');
  await projectSearch.press('Enter');

  // 6. After Project pick, focus should cascade to the composer textarea.
  const composer = draft.getByLabel('Task message', { exact: true });
  await expect(composer).toBeFocused({ timeout: 5000 });

  // 7. Confirm the cascade wrote the chosen ids into the model by typing
  //    (sanity: the textarea still accepts input and the pickers display the
  //    chosen labels).
  await composer.type('cascade test');
  await expect(agentTrigger).toContainText(/Mariner/);
  await expect(projectTrigger).toContainText(/Seafood ops/);

  console.log('draft pickers: cascade Agent→Project→composer works with search filter');
} finally {
  await browser.close();
}
