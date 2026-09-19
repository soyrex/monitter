import { webkit, expect } from '@playwright/test';

const browser = await webkit.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1200, height: 780 } });
page.setDefaultTimeout(10_000);
try {
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18474', { waitUntil: 'domcontentloaded', timeout: 60_000 });

  const widget = page.locator('.sidebar-clock-widget');
  await widget.waitFor({ state: 'visible', timeout: 60_000 });
  await expect(widget).toHaveAttribute('aria-label', 'Clock and Monitter usage widget');
  await expect(widget.locator('.widget-toggle > span')).toContainText('LOCAL TIME ·');
  await expect(widget.getByText('CPU', { exact: true })).toBeVisible();
  await expect(widget.getByText('RAM', { exact: true })).toBeVisible();
  await expect.poll(async () => widget.locator('.sparkline polyline').evaluateAll(lines => lines.length === 2 && lines.every(line => (line.getAttribute('points') || '').trim().split(/\s+/).filter(Boolean).length >= 2)), { timeout: 8_000 }).toBe(true);

  for (const label of ['CPU', 'RAM']) {
    const row = widget.locator('.metric-row').filter({ hasText: label });
    const graph = await row.locator('.sparkline').boundingBox();
    const name = await row.getByText(label, { exact: true }).boundingBox();
    const value = await row.locator('strong').boundingBox();
    if (!graph || !name || !value || graph.x >= name.x || name.x >= value.x) {
      throw new Error(`${label} row is not ordered graph, label, value.`);
    }
  }

  await expect(widget.locator('.metric-row').first()).toHaveClass(/level-low/);
  await page.screenshot({ path: 'output/sidebar-clock-sparklines.png' });
  for (const level of ['medium', 'high', 'critical']) {
    await expect(widget.locator('.metric-row').first()).toHaveClass(new RegExp(`level-${level}`), { timeout: 5_000 });
    await expect(widget.locator('.metric-row').last()).toHaveClass(new RegExp(`level-${level}`), { timeout: 5_000 });
  }

  await page.reload({ waitUntil: 'domcontentloaded' });
  await page.getByRole('separator', { name: 'Resize main sidebar' }).press('Enter');
  await expect(page.locator('.app-shell')).toHaveClass(/sidebar-collapsed/);
  await expect(page.locator('.sidebar-clock-widget')).toHaveCount(0);
  const compactUsage = page.getByRole('button', { name: 'Open provider usage', exact: true });
  await expect(compactUsage).toBeVisible();
  const footer = page.locator('.sidebar-footer');
  await expect(footer.getByRole('button')).toHaveCount(1);
  await expect(footer.getByRole('button', { name: 'Preferences', exact: true })).toBeVisible();
  await compactUsage.click();
  await expect(page.locator('.app-shell')).not.toHaveClass(/sidebar-collapsed/);
  await expect(page.locator('.usage-rings')).toBeVisible();
  console.log('Sidebar clock: expanded view retains clock and sparklines; compact rail uses Usage and Settings controls only.');
} finally {
  await browser.close();
}
