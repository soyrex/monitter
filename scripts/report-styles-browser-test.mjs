// Offline browser verification for the portable MCP report CSS. No app/profile mutation.
import { chromium, webkit } from '@playwright/test';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
const css = await readFile(new URL('../src-tauri/src/report.css', import.meta.url), 'utf8');
const engine = process.env.REPORT_BROWSER || 'chromium';
assert.ok(['chromium', 'webkit'].includes(engine), 'Unsupported browser engine');
const output = new URL(`../artifacts/report-styles/${engine}/`, import.meta.url);
await mkdir(output, { recursive: true });
const palettes = {
  light: { paper:'#f6f3ec',panel:'#fffdf7',sidebar:'#eeeae1',line:'#d4cfc4',soft:'#eae6dd',code:'#eeeae1',ink:'#292824',muted:'#666259',accent:'#176f88' },
  dark: { paper:'#17191e',panel:'#20232a',sidebar:'#1b1d23',line:'#41454f',soft:'#292d35',code:'#111318',ink:'#eeeef0',muted:'#afb2bb',accent:'#78c8e1' },
};
const browser = await ({ chromium, webkit })[engine].launch({ headless: true });
const results = [];
try {
  for (const [mode, tokens] of Object.entries(palettes)) {
    let baseHtml;
    for (const width of [375,768,1440]) {
      const page = await browser.newPage({ viewport: { width, height: 1000 } });
      const vars = Object.entries(tokens).map(([key,value])=>`--report-${key}:${value}`).join(';');
      const html = `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Monitter report CSS fixture</title><style>.monitter-report{${vars};--report-font-family:system-ui,sans-serif;--report-mono-font-family:ui-monospace,monospace;--report-font-size:16px;color-scheme:${mode}}${css}</style><body class="monitter-report"><main class="report-container"><header class="report-header"><p class="report-eyebrow">Report style fixture</p><h1>A clear view of the project</h1><p class="report-lead">A responsive report built with the Monitter theme tokens.</p><div class="report-meta"><span>14 October 2026</span><span class="report-badge">Draft</span></div></header><section class="report-grid" aria-label="Summary"><article class="report-card"><p class="report-eyebrow">Completed</p><strong class="report-metric">24</strong><p>Tasks ready for review.</p></article><article class="report-card"><p class="report-eyebrow">Next checkpoint</p><strong class="report-metric">Friday</strong><p>Confirm scope with the project owner.</p></article><article class="report-card"><h2>Context</h2><p>Long content wraps safely: ${'long-project-name-'.repeat(12)}</p></article></section><h2>Findings</h2><aside class="report-callout"><strong>Decision needed</strong><p>Choose the next milestone after reviewing the evidence.</p></aside><div class="report-table-wrap" tabindex="0" aria-label="Task table"><table><caption>Task comparison</caption><thead><tr><th scope="col">Area</th><th scope="col">Owner</th><th scope="col">Progress</th></tr></thead><tbody><tr><td>Interface</td><td>Design team</td><td class="report-numeric">80%</td></tr><tr><td>Integration</td><td>Engineering</td><td class="report-numeric">65%</td></tr></tbody></table></div><pre tabindex="0"><code>${'example_of_a_long_code_line_'.repeat(15)}</code></pre><details><summary>Additional notes</summary><p>Expandable context remains usable on smaller screens.</p></details><p><a href="#notes">Read the notes</a></p><footer id="notes" class="report-footer">Synthetic fixture for responsive style verification.</footer></main></body></html>`;
      baseHtml = html;
      await page.setContent(html);
      const measures = await page.evaluate(() => ({
        viewport: innerWidth, documentWidth: document.documentElement.scrollWidth,
        background: getComputedStyle(document.body).backgroundColor,
        columns: getComputedStyle(document.querySelector('.report-grid')).gridTemplateColumns.split(' ').length,
        codeScrolls: document.querySelector('pre').scrollWidth > document.querySelector('pre').clientWidth,
      }));
      assert.ok(measures.documentWidth <= width, `${mode}/${width}: page overflow ${JSON.stringify(measures)}`);
      assert.ok(measures.codeScrolls, 'Long code should scroll inside its container');
      if (width === 375) assert.equal(measures.columns, 1);
      if (width === 1440) assert.equal(measures.columns, 3);
      const hex = tokens.paper.slice(1); const expected = `rgb(${parseInt(hex.slice(0,2),16)}, ${parseInt(hex.slice(2,4),16)}, ${parseInt(hex.slice(4,6),16)})`;
      assert.equal(measures.background, expected);
      await page.screenshot({ path: new URL(`${mode}-${width}.png`, output).pathname, fullPage:true });
      await writeFile(new URL(`${mode}-${width}.html`, output), html);
      await page.emulateMedia({ media:'print' });
      assert.equal(await page.locator('body').evaluate(el=>getComputedStyle(el).backgroundColor), 'rgb(255, 255, 255)');
      results.push({mode,width,...measures,print:'passed'});
      await page.close();
    }
    // Navigation is opt-in: retain the standalone report cases above.
    for (const width of [375, 1023, 1024, 1440]) {
      const page = await browser.newPage({ viewport: { width, height: 480 } });
      const links = Array.from({ length: 32 }, (_, i) => `<a href="#${i === 0 ? 'findings' : 'notes'}">${i === 0 ? 'Findings' : i === 1 ? 'A longer section title with enough words to wrap safely' : `Section ${i + 1}`}</a>`).join('');
      const html = baseHtml.replace('<main class="report-container">', `<div class="report-layout"><aside class="report-sidebar"><p class="report-eyebrow">Contents</p><nav class="report-toc" aria-label="Report sections">${links}</nav></aside><main class="report-container">`)
        .replace('<h2>Findings</h2>', '<h2 id="findings">Findings</h2>')
        .replace('</main>', '</main></div>');
      await page.setContent(html);
      const geometry = await page.evaluate(() => {
        const sidebar = document.querySelector('.report-sidebar');
        const main = document.querySelector('main');
        const navLinks = [...document.querySelectorAll('.report-toc a')];
        const side = sidebar.getBoundingClientRect(), content = main.getBoundingClientRect();
        return {
          documentWidth: document.documentElement.scrollWidth,
          sideTop: side.top, sideBottom: side.bottom, sideRight: side.right,
          contentLeft: content.left, contentTop: content.top,
          rows: new Set(navLinks.map(a => Math.round(a.getBoundingClientRect().top))).size,
          rounded: navLinks.every(a => parseFloat(getComputedStyle(a).borderTopLeftRadius) > 0),
          sidebarScrolls: sidebar.scrollHeight > sidebar.clientHeight,
        };
      });
      const label = `${mode}/${width}/navigation`;
      assert.ok(geometry.documentWidth <= width, `${label}: horizontal overflow`);
      if (width >= 1024) {
        assert.ok(geometry.sideRight <= geometry.contentLeft + 1, `${label}: sidebar overlaps content`);
        assert.ok(geometry.sidebarScrolls, `${label}: short sidebar must scroll`);
        await page.evaluate(() => window.scrollTo(0, 250));
        assert.ok(await page.evaluate(() => scrollY > 0), `${label}: fixture must permit body scrolling`);
        const after = await page.locator('.report-sidebar').boundingBox();
        assert.ok(Math.abs(after.y - geometry.sideTop) <= 1, `${label}: sidebar moved with document`);
        await page.locator('.report-sidebar').evaluate(el => { el.scrollTop = el.scrollHeight; });
        const reachable = await page.evaluate(() => {
          const side = document.querySelector('.report-sidebar');
          const last = document.querySelector('.report-toc a:last-child').getBoundingClientRect();
          const box = side.getBoundingClientRect();
          return side.scrollTop > 0 && last.top >= box.top && last.bottom <= box.bottom + 1;
        });
        assert.ok(reachable, `${label}: last navigation link unreachable`);
      } else {
        assert.ok(geometry.sideBottom <= geometry.contentTop + 1, `${label}: navigation overlaps content`);
        assert.ok(geometry.rows > 1 && geometry.rounded, `${label}: pill links should wrap across rows`);
        await page.evaluate(() => window.scrollTo(0, 0));
        // WebKit follows macOS's default Tab policy, which can skip links.
        // Focus the anchor explicitly, then verify native keyboard activation.
        await page.locator('.report-toc a').first().focus();
        assert.equal(await page.evaluate(() => document.activeElement?.textContent), 'Findings', `${label}: first link keyboard focus`);
        await page.keyboard.press('Enter');
        await page.waitForFunction(() => location.hash === '#findings');
        assert.ok(await page.locator('#findings').evaluate(el => {
          const box = el.getBoundingClientRect();
          return box.top >= -1 && box.top < innerHeight;
        }), `${label}: keyboard anchor target is offscreen`);
      }
      await page.screenshot({ path: new URL(`${mode}-${width}-navigation.png`, output).pathname, fullPage:true });
      await writeFile(new URL(`${mode}-${width}-navigation.html`, output), html);
      await page.emulateMedia({ media:'print' });
      assert.equal(await page.locator('.report-sidebar').evaluate(el => el.getBoundingClientRect().height), 0, `${label}: print sidebar visible`);
      const printLayout = await page.locator('.report-layout').evaluate(el => ({
        left: parseFloat(getComputedStyle(el).paddingLeft), right: parseFloat(getComputedStyle(el).paddingRight),
      }));
      assert.deepEqual(printLayout, { left:0, right:0 }, `${label}: print layout remains indented`);
      results.push({ mode, width, fixture:'navigation', ...geometry, print:'passed' });
      await page.close();
    }

  }
} finally { await browser.close(); }
await writeFile(new URL('browser-results.json', output), JSON.stringify(results,null,2)+'\n');
console.log(`Passed ${engine}: ${results.length} responsive light/dark cases and print checks.`);
