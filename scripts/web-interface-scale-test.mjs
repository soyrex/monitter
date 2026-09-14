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

  const samples = [];
  for (const scale of [80, 125, 200]) {
    await page.evaluate(value => {
      const qa = window.__MONITTER_QA__;
      const state = qa.snapshot();
      state.settings.interfaceScale = value;
      qa.setSnapshot(state);
    }, scale);
    await expect.poll(() => page.evaluate(() => getComputedStyle(document.querySelector('.app-shell')).zoom)).toBe(String(scale / 100));
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
  expect(errors).toEqual([]);
  await page.close();
  return label;
}

for (const engine of [chromium, webkit]) {
  const browser = await engine.launch({ headless: true });
  try {
    const checked = [];
    checked.push(await verifyViewport(browser, { width: 390, height: 844 }, 'mobile'));
    checked.push(await verifyViewport(browser, { width: 1200, height: 800 }, 'desktop browser'));
    console.log(`${engine.name()}: ${checked.join(' and ')} interfaces scale 80–200% while fitting the viewport`);
  } finally {
    await browser.close();
  }
}
