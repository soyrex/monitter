import { webkit, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const browser = await webkit.launch();
try {
  for (const mobile of [false, true]) {
    const page = await browser.newPage({ viewport: mobile ? {width:390,height:844} : {width:1100,height:800}, isMobile:mobile, hasTouch:mobile });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    // Fixture IDs also need to work at a non-secure LAN origin.
    await page.addInitScript('crypto.randomUUID ??= () => "terminal-fixture-id";' + readFileSync('scripts/ui-fixture.js', 'utf8'));
    await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18438/');
    await expect(page.locator('.sidebar-tabs')).toBeVisible({timeout:60000});
    if (mobile) {
      await page.keyboard.press('Meta+p');
      await page.getByRole('button', {name:/New terminal/}).click();
    } else {
      await page.getByRole('button', {name:'New terminal',exact:true}).click();
    }
    await expect(page.locator('.terminal-pane .xterm')).toBeVisible();
    await expect(page.locator('.terminal-pane')).toContainText('Monitter browser terminal');
    await page.locator('.xterm-helper-textarea').pressSequentially('hello');
    await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.some(call => call.method === 'writeTerminal'))).toBe(true);
    expect(errors).toEqual([]);
    await page.close();
  }
  console.log('WebKit desktop and mobile terminal rendering/input pass.');
} finally { await browser.close(); }
