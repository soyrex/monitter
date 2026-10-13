import { chromium, webkit, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';
const output = 'artifacts/mobile-preferences';
mkdirSync(output, { recursive: true });
const browserType = process.env.MONITTER_TEST_BROWSER === 'webkit' ? webkit : chromium;
const browser = await browserType.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
const errors = [], results = [];
page.on('pageerror', error => { errors.push(error.message); console.error(error.stack); });
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18473';
async function choose(label, id) {
  const picker = page.getByRole('button', { name: 'Preferences category', exact: true });
  if (await picker.isVisible()) { await picker.click(); await page.locator('.phone-category-list').getByRole('button',{name:label,exact:true}).click(); }
  else await page.getByRole('navigation', { name: 'Settings categories' }).getByRole('button', { name: label, exact: true }).click();
  await expect(page.locator('.settings-header h1')).toHaveText(label);
}
try {
  await page.addInitScript(() => { window.__TAURI_INTERNALS__ = { transformCallback: () => 1, invoke: async () => 1 }; });
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(url, { timeout: 60000 });
  await expect(page.getByRole('button', { name: 'Open global overview', exact: true })).toBeVisible({timeout:60000});
  await page.keyboard.press('Meta+,');
  const nav = page.getByRole('navigation', { name: 'Settings categories' });
  for (const group of ['Workspace', 'Agents & integrations', 'Security & access']) await expect(nav.locator('.nav-heading').filter({ hasText: group })).toHaveCount(1);
  const icons = await nav.locator('.category-list button svg').evaluateAll(nodes => nodes.map(node => node.innerHTML));
  expect(icons.length).toBeGreaterThanOrEqual(12); expect(new Set(icons).size).toBe(icons.length);
  await choose('Layout & panes', 'interface');
  for (const name of ['Interface scale', 'Interface density', 'Show tab close buttons']) await expect(page.getByRole(name === 'Show tab close buttons' ? 'switch' : 'slider',{ name,exact:true })).toBeVisible();
  await expect(page.locator('#motion-heading')).toBeVisible();
  await page.reload();
  await expect(page.locator('.settings-header h1')).toHaveText('Layout & panes');
  results.push('Distinct grouped icons and persisted Layout & panes route');
  await choose('Keyboard & focus', 'behaviour');
  await expect(page.getByRole('switch', { name: 'Default to steering busy agents' })).toHaveCount(0);
  await choose('Conversation', 'conversation');
  await page.getByRole('switch', { name: 'Default to steering busy agents' }).check();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().settings.busyMessageMode)).toBe('steer');
  await expect(page.getByRole('switch', { name: 'Enable project boards' })).toHaveCount(0);
  await choose('Collaboration', 'collaboration');
  await page.getByRole('switch', { name: 'Enable project boards' }).check();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().settings.projectBoardEnabled)).toBe(true);
  await expect(page.getByRole('button',{name:'Save guidance',exact:true})).toBeEnabled();
  await expect(page.getByRole('switch', { name: 'Enable project boards' })).toBeChecked();
  await page.getByLabel('Default project-board guidance', { exact: true }).fill('Coordinate fixture work.');
  await page.evaluate(() => { const q=window.__MONITTER_QA__,s=q.snapshot();s.settings.showToolActivity=false;q.setSnapshot(s); });
  await expect(page.getByLabel('Default project-board guidance', { exact: true })).toHaveValue('Coordinate fixture work.');
  await page.getByRole('button', { name: 'Save guidance', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().settings.boardGuidanceText)).toBe('Coordinate fixture work.');
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().settings.projectBoardEnabled)).toBe(true);
  await page.evaluate(() => { const q=window.__MONITTER_QA__,s=q.snapshot();s.settings.boardGuidanceText='External fixture guidance';q.setSnapshot(s); });
  await expect(page.getByLabel('Default project-board guidance', { exact: true })).toHaveValue('External fixture guidance');
  await page.reload(); await expect(page.locator('.settings-header h1')).toHaveText('Collaboration');
  results.push('Moved busy-agent and board settings save through the fixture bridge; Collaboration restores');
  await choose('Profile', 'profile');
  const name = page.getByRole('textbox',{name:'Your name',exact:true});
  await name.fill('a'.repeat(81)); await name.press('Tab');
  await expect(name).toHaveAttribute('aria-invalid', 'true');
  expect(await page.evaluate(() => window.__MONITTER_QA__.snapshot().settings.userName)).toBeUndefined();
  await name.fill('Alex fixture'); await name.press('Tab');
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().settings.userName)).toBe('Alex fixture');
  await page.evaluate(() => { const bridge = window.__MONITTER_BRIDGE__, original = bridge.saveSettings; bridge.saveSettings = async () => { bridge.saveSettings = original; throw Error('Preferences save failure fixture'); }; });
  await name.fill('Recovered fixture'); await name.press('Tab');
  await expect(page.locator('.settings-pane').getByRole('alert')).toContainText('Preferences save failure fixture');
  await name.fill('Recovery fixture'); await name.press('Tab');
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().settings.userName)).toBe('Recovery fixture');
  await expect(page.locator('.settings-pane').getByRole('alert')).toHaveCount(0);
  results.push('Profile validation prevents invalid save; failures stay visible and recover');
  await choose('Agents', 'agents'); await expect(page.getByRole('textbox', { name: 'Find agents' })).toBeVisible();
  const categories = [['Appearance','appearance'],['Layout & panes','interface'],['Fonts & text','typography'],['Keyboard & focus','behaviour'],['Conversation','conversation'],['Collaboration','collaboration'],['Profile','profile']];
  for (const width of [320,390,768]) {
    await page.setViewportSize({ width, height: 900 });
    await page.keyboard.press('Meta+,');
    await expect(page.locator('.settings-pane')).toBeVisible();
    for (const [label,id] of categories) {
      await choose(label,id);
      const bounds = await page.locator('.settings-pane').evaluate(element => ({width:element.clientWidth,scroll:element.scrollWidth,documentWidth:document.documentElement.clientWidth,documentScroll:document.documentElement.scrollWidth,overflow:[...element.querySelectorAll('.setting-card')].some(card=>card.scrollWidth>card.clientWidth+1)}));
      expect(bounds.scroll, `${width} ${label} pane overflow`).toBeLessThanOrEqual(bounds.width+1);
      expect(bounds.documentScroll, `${width} ${label} document overflow`).toBeLessThanOrEqual(bounds.documentWidth+1);
      expect(bounds.overflow, `${width} ${label} card overflow`).toBe(false);
      if (width <=390) {
        const picker = page.getByRole('button',{name:'Preferences category',exact:true});
        await expect(picker).toBeVisible(); expect((await picker.boundingBox()).height).toBeGreaterThanOrEqual(44);
      }
    }
    await choose('Layout & panes','interface');
    await page.screenshot({path:`${output}/${process.env.MONITTER_TEST_BROWSER || 'chromium'}-${width}.png`});
    if(width === 390) {
      await page.getByRole('button',{name:'Preferences category',exact:true}).click();
      expect(await page.locator('.phone-category-list button svg').count()).toBe(icons.length);
      await page.screenshot({path:`${output}/${process.env.MONITTER_TEST_BROWSER || 'chromium'}-390-menu.png`});
      await page.keyboard.press('Escape');
      await expect(page.getByRole('button',{name:'Preferences category',exact:true})).toBeFocused();
      await choose('Profile','profile');
      await page.screenshot({path:`${output}/${process.env.MONITTER_TEST_BROWSER || 'chromium'}-390-profile.png`});
    }
  }
  results.push('320/390/768 layouts fit at fixture 125% scale; phones expose a labeled 44px icon category menu');
  expect(errors).toEqual([]);
  writeFileSync(`${output}/layout-${process.env.MONITTER_TEST_BROWSER || 'chromium'}.json`,JSON.stringify({results,errors},null,2));
  console.log(JSON.stringify({results,errors},null,2));
} catch(error) {
  await page.screenshot({path:`${output}/failure-${process.env.MONITTER_TEST_BROWSER || 'chromium'}.png`});
  throw error;
} finally { await browser.close(); }
