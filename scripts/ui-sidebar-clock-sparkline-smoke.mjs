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
  const compact = page.locator('.sidebar-clock-widget.compact');
  await expect(compact.locator('.compact-time span')).not.toBeEmpty();
  await expect(compact.getByText('CPU', { exact: true })).toBeVisible();
  await expect(compact.getByText('RAM', { exact: true })).toBeVisible();
  await expect.poll(async () => compact.locator('.sparkline polyline').count(), { timeout: 6_000 }).toBe(2);
  const compactBox = await compact.boundingBox();
  const contentBoxes = await compact.locator('.clock-face, .metric-row').evaluateAll(nodes => nodes.map(node => {
    const box = node.getBoundingClientRect(); return { left: box.left, right: box.right };
  }));
  if (!compactBox || contentBoxes.some(box => box.left < compactBox.x - .5 || box.right > compactBox.x + compactBox.width + .5)) {
    throw new Error('Compact clock or metrics overflow the collapsed rail.');
  }
  // A transformed, device-scaled rail can round scrollWidth one CSS pixel above
  // clientWidth even when the rendered text remains fully inside its box.
  const clippedReadings = await compact.locator('.metric-row strong').evaluateAll(nodes => nodes.some(node => node.scrollWidth > node.clientWidth + 1));
  if (clippedReadings) throw new Error('Compact metric readings are clipped.');
  await page.screenshot({ path: 'output/sidebar-clock-compact.png' });
  console.log('Sidebar clock: expanded and compact layouts, timezone, live sparklines, and green/yellow/orange/red levels passed.');
} finally {
  await browser.close();
}
