import { chromium, webkit, expect } from '@playwright/test';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18465';

async function verifyViewport(browser, viewport, label) {
  const page = await browser.newPage({ viewport });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await expect(page.locator('.app-shell')).toBeVisible({ timeout: 30_000 });
  const viewer = viewport.width <= 760 ? 'mobile' : 'desktop';
  const activeKey = `monitter.interface-scale.v1:${viewer}`;
  const inactiveKey = `monitter.interface-scale.v1:${viewer === 'mobile' ? 'desktop' : 'mobile'}`;

  const samples = [];
  for (const scale of [80, 125, 200]) {
    await page.evaluate(({key,value}) => {
      localStorage.setItem(key, String(value));
      window.dispatchEvent(new StorageEvent('storage', { key, newValue: String(value) }));
    }, {key:activeKey,value:scale});
    await expect.poll(() => page.evaluate(() => getComputedStyle(document.querySelector('.app-shell')).zoom)).toBe(String(scale / 100));
    await expect.poll(() => page.evaluate(() => {
      const shell = document.querySelector('.app-shell').getBoundingClientRect();
      return { width: Math.round(shell.width), height: Math.round(shell.height) };
    })).toEqual({ width: viewport.width, height: viewport.height });
    samples.push(await page.evaluate(() => {
      const shell = document.querySelector('.app-shell').getBoundingClientRect();
      const brand = document.querySelector('.brand').getBoundingClientRect();
      const button = document.querySelector('.view-toggle').getBoundingClientRect();
      return {
        shell: { left: shell.left, top: shell.top, width: shell.width, height: shell.height },
        brandHeight: brand.height,
        buttonWidth: button.width,
      };
    }));
  }

  for (const sample of samples) {
    expect(Math.abs(sample.shell.left)).toBeLessThan(1);
    expect(Math.abs(sample.shell.top)).toBeLessThan(1);
    expect(Math.abs(sample.shell.width - viewport.width)).toBeLessThan(1);
    expect(Math.abs(sample.shell.height - viewport.height)).toBeLessThan(1);
  }
  expect(samples[0].brandHeight).toBeLessThan(samples[1].brandHeight);
  expect(samples[1].brandHeight).toBeLessThan(samples[2].brandHeight);
  expect(samples[0].buttonWidth).toBeLessThan(samples[1].buttonWidth);
  expect(samples[1].buttonWidth).toBeLessThan(samples[2].buttonWidth);
  await page.evaluate(({key}) => {
    localStorage.setItem(key, '80');
    window.dispatchEvent(new StorageEvent('storage', { key, newValue: '80' }));
  }, {key:inactiveKey});
  await expect.poll(() => page.evaluate(() => getComputedStyle(document.querySelector('.app-shell')).zoom)).toBe('2');
  expect(await page.evaluate(() => window.__MONITTER_QA__.snapshot().settings.interfaceScale)).toBe(125);
  expect(errors).toEqual([]);
  await page.close();
  return label;
}

async function verifyViewerSwitch(browser) {
  const page = await browser.newPage({ viewport: { width: 1200, height: 800 } });
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.addInitScript(() => {
    localStorage.setItem('monitter.interface-scale.v1:desktop', '80');
    localStorage.setItem('monitter.interface-scale.v1:mobile', '200');
  });
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await expect(page.locator('.app-shell')).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => page.evaluate(() => getComputedStyle(document.querySelector('.app-shell')).zoom)).toBe('0.8');
  await page.setViewportSize({ width: 390, height: 844 });
  await expect.poll(() => page.evaluate(() => getComputedStyle(document.querySelector('.app-shell')).zoom)).toBe('2');
  await page.setViewportSize({ width: 1200, height: 800 });
  await expect.poll(() => page.evaluate(() => getComputedStyle(document.querySelector('.app-shell')).zoom)).toBe('0.8');
  await page.close();
}

async function verifyDedicatedMobileViewer(browser) {
  const page = await browser.newPage({ viewport: { width: 390, height: 844 } });
  await page.addInitScript(() => {
    localStorage.setItem('monitter.interface-scale.v1:desktop', '80');
    localStorage.setItem('monitter.interface-scale.v1:mobile', '175');
  });
  await page.goto(new URL('mobile', url).href, { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await expect(page.getByRole('button', { name: 'Scan desktop code' })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => page.evaluate(() => getComputedStyle(document.querySelector('.mobile')).getPropertyValue('--viewer-interface-scale').trim())).toBe('1.75');
  const bounds = await page.locator('.mobile').evaluate(element => {
    const box = element.getBoundingClientRect();
    return { width: Math.round(box.width), height: Math.round(box.height) };
  });
  expect(bounds.width).toBe(390);
  expect(bounds.height).toBe(844);
  expect(await page.evaluate(() => localStorage.getItem('monitter.interface-scale.v1:desktop'))).toBe('80');
  await page.close();
}

for (const engine of [chromium, webkit]) {
  const browser = await engine.launch({ headless: true });
  try {
    const checked = [];
    checked.push(await verifyViewport(browser, { width: 390, height: 844 }, 'mobile'));
    checked.push(await verifyViewport(browser, { width: 1200, height: 800 }, 'desktop browser'));
    await verifyViewerSwitch(browser);
    await verifyDedicatedMobileViewer(browser);
    console.log(`${engine.name()}: ${checked.join(' and ')} viewer scales stay independent across the responsive breakpoint`);
  } finally {
    await browser.close();
  }
}
