import { chromium, webkit, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';

const browserName = process.env.MONITTER_TEST_BROWSER || 'chromium';
const browser = await (browserName === 'webkit' ? webkit : chromium).launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
const output = 'artifacts/preferences-search';
mkdirSync(output, { recursive: true });
const errors = [], checks = [];
page.on('pageerror', error => errors.push(error.message));
const search = page.getByRole('combobox', { name: 'Search preferences', exact: true });
const options = page.getByRole('listbox', { name: 'Preference search results', exact: true }).getByRole('option');
const saveCount = () => page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'saveSettings').length);
try {
  await page.addInitScript(() => { window.__TAURI_INTERNALS__ = { transformCallback: () => 1, invoke: async () => 1 }; });
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18474', { timeout: 60000 });
  await expect(page.getByRole('button', { name: 'Open global overview', exact: true })).toBeVisible({ timeout: 60000 });
  await page.keyboard.press('Meta+,');
  await expect(search).toBeVisible();
  const initialSaves = await saveCount();
  for (const query of ['font size', 'reduce motion', 'transparncy', 'guidance', 'zzznonexistentzzz']) await search.fill(query);
  await expect(options).toHaveCount(0);
  await expect(page.getByRole('status').filter({ hasText: /No preferences found/ })).toBeVisible();
  expect(await saveCount()).toBe(initialSaves);
  await search.press('Escape');
  await expect(search).toHaveValue('');
  await expect(search).toBeFocused();
  await expect(page.locator('.settings-pane')).toBeVisible();
  checks.push('Search is local and non-mutating; no-results and Escape clear work');

  await search.fill('font size');
  await expect(options.first()).toContainText(/font size/i);
  const firstId = await search.getAttribute('aria-activedescendant');
  await search.press('ArrowDown');
  expect(await search.getAttribute('aria-activedescendant')).not.toBe(firstId);
  await search.press('ArrowUp');
  await expect(search).toHaveAttribute('aria-activedescendant', firstId);
  await search.press('Enter');
  await expect(page.locator('.settings-header h1')).toHaveText('Fonts & text');
  await expect(page.locator('#fonts-heading')).toBeFocused();
  await expect(page.locator('.search-highlight')).toContainText('Fonts');
  await expect(search).toHaveValue('');
  checks.push('Ranked results and keyboard selection jump to the matching card');

  await search.fill('guidance');
  await options.first().click();
  const guidance = page.getByLabel('Default project-board guidance', { exact: true });
  await guidance.fill('Unsaved search-navigation draft');
  await search.fill('transparncy');
  await expect(options.first()).toContainText('Window transparency');
  await options.first().click();
  await expect(page.locator('.settings-header h1')).toHaveText('Appearance');
  await expect(page.locator('#window-surface-heading')).toBeFocused();
  await search.fill('guidance');
  await search.press('Enter');
  await expect(guidance).toHaveValue('Unsaved search-navigation draft');
  expect(await saveCount()).toBe(initialSaves);
  checks.push('Typo matching works; navigation preserves an unsaved guidance draft');

  await search.fill('Workspace connection');
  await expect(options.filter({ hasText: 'Workspace connection' })).toHaveCount(0);
  await page.getByRole('button', { name: 'Clear preferences search', exact: true }).click();
  await expect(search).toBeFocused();
  checks.push('Unavailable native phone connection controls stay out of results');

  for (const width of [320, 390, 768]) {
    await page.setViewportSize({ width, height: 900 });
    await search.fill('font');
    await expect(options.first()).toBeVisible();
    const bounds = await page.locator('.settings-pane').evaluate(element => ({ width: element.clientWidth, scroll: element.scrollWidth, docWidth: document.documentElement.clientWidth, docScroll: document.documentElement.scrollWidth }));
    expect(bounds.scroll).toBeLessThanOrEqual(bounds.width + 1);
    expect(bounds.docScroll).toBeLessThanOrEqual(bounds.docWidth + 1);
    await page.screenshot({ path: `${output}/${browserName}-${width}.png` });
    await search.fill('reduce motion');
    await options.first().click();
    await expect(page.locator('.settings-header h1')).toHaveText('Layout & panes');
    await expect(page.locator('#motion-heading')).toBeFocused();
    await expect(page.getByRole('button', { name: 'Preferences category', exact: true })).toBeVisible();
  }
  checks.push('Search fits 320, 390 and 768px viewports at the fixture interface scale');
  expect(errors).toEqual([]);
  writeFileSync(`${output}/${browserName}.json`, JSON.stringify({ checks, errors }, null, 2));
  console.log(JSON.stringify({ checks, errors }, null, 2));
} catch (error) {
  await page.screenshot({ path: `${output}/${browserName}-failure.png` });
  throw error;
} finally { await browser.close(); }
