import assert from 'node:assert/strict';
import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { chromium, webkit, expect } from '@playwright/test';
import { createServer } from 'node:http';

const root = process.cwd();
const result = await build({ configFile: false, root, plugins: [svelte()], resolve: { alias: [
  { find: '$lib/bridge', replacement: `${root}/scripts/fixtures/attachment-image-bridge.ts` },
  { find: '$lib', replacement: `${root}/src/lib` },
] }, build: { write: false, rollupOptions: { input: 'scripts/fixtures/attachment-image-entry.js' } } });
const files = new Map(result.output.map(file => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]));
const entry = result.output.find(file => file.type === 'chunk' && file.isEntry);
files.set('/', `<!doctype html><style>body{margin:20px;--panel:white;--ink:black;--soft:#eee;--line:#ddd}</style>${result.output.filter(file => file.fileName.endsWith('.css')).map(file => `<link rel="stylesheet" href="/${file.fileName}">`).join('')}<div id="app"></div><script type="module" src="/${entry.fileName}"></script>`);
const server = createServer((req, res) => {
  const body = files.get(req.url);
  if (body === undefined) { res.writeHead(404); res.end(); return; }
  res.setHeader('Content-Type', req.url === '/' ? 'text/html' : req.url.endsWith('.css') ? 'text/css' : 'text/javascript'); res.end(body);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
try {
  for (const engine of [webkit, chromium]) {
    const browser = await engine.launch({ headless: true });
    try {
      const page = await browser.newPage({ viewport: { width: 1000, height: 700 } });
      const errors = []; page.on('pageerror', error => errors.push(error.message));
      await page.goto(`http://127.0.0.1:${server.address().port}/`);
      const sizes = await page.evaluate(async () => {
        const canvas = document.createElement('canvas'); canvas.width = 2400; canvas.height = 1400;
        const ctx = canvas.getContext('2d'); ctx.fillStyle = 'white'; ctx.fillRect(0,0,2400,1400); ctx.fillStyle = 'black'; ctx.font = '28px monospace';
        for (let y=40;y<1400;y+=40) ctx.fillText(`Screenshot text row ${y}: preserve original pixels`, 20, y);
        const blob = await new Promise(resolve => canvas.toBlob(resolve, 'image/png'));
        const file = new File([blob], 'screenshot.png', { type:'image/png' });
        const qa = window.__IMAGE_QA__; qa.file = await qa.readBrowserFile(file); qa.preview = await qa.thumbnail(file); qa.reads = [];
        qa.read = async id => { qa.reads.push(id); return qa.file; };
        qa.attachment = { id:'stored-image', name:'screenshot.png', mimeType:'image/png', size:file.size, path:'/workspace/.monitter/attachments/original.png', previewDataUrl:qa.preview };
        await qa.show([qa.attachment]);
        const preview = new Image(); preview.src=qa.preview; await preview.decode();
        return { width:preview.naturalWidth, height:preview.naturalHeight, originalBytes:file.size };
      });
      assert.equal(sizes.width,192); assert.ok(sizes.originalBytes>0);
      assert.equal(await page.evaluate(() => window.__IMAGE_QA__.reads.length),0,'opening transcript never fetches full images');
      const trigger = page.getByRole('button',{name:'View full-size screenshot.png'});
      await trigger.click();
      const full = page.locator('dialog img');
      await expect(full).toBeVisible();
      await expect.poll(() => full.evaluate(img => img.naturalWidth)).toBe(2400);
      assert.equal(await full.evaluate(img => img.naturalHeight),1400);
      assert.equal(await full.evaluate(img => img.src === `data:image/png;base64,${window.__IMAGE_QA__.file.dataBase64}`),true,'viewer uses exact original response');
      await page.getByRole('button',{name:'View actual size'}).click();
      await expect(full).toHaveClass(/\bactual-size\b/);
      await page.getByRole('button',{name:'Close image preview'}).click();
      await expect(trigger).toBeFocused();
      await page.evaluate(() => { window.__IMAGE_QA__.read = async () => { throw new Error('Original file unavailable'); }; });
      await trigger.click();
      await expect(page.getByRole('alert')).toContainText('Original file unavailable');
      await expect(page.locator('dialog')).toHaveCount(0);
      await page.evaluate(async () => {
        const qa = window.__IMAGE_QA__;
        qa.read = id => id === 'stored-image' ? new Promise(resolve => { qa.resolveOld = resolve; }) : Promise.resolve(qa.file);
        await qa.show([qa.attachment, {...qa.attachment, id:'second-image', name:'second.png'}]);
      });
      await trigger.click();
      await expect(page.getByRole('status')).toContainText('Loading original image');
      await page.getByRole('button',{name:'View full-size second.png'}).click();
      await expect(page.getByRole('dialog',{name:'second.png'})).toBeVisible();
      await page.evaluate(() => window.__IMAGE_QA__.resolveOld(window.__IMAGE_QA__.file));
      await expect(page.getByRole('dialog',{name:'second.png'})).toBeVisible();
      await page.getByRole('button',{name:'Close image preview'}).click();
      await page.evaluate(async () => {
        const qa = window.__IMAGE_QA__; qa.read = async () => { throw new Error('Should not read inline tool image'); };
        await qa.show([{...qa.attachment,path:'Generated by Codex',previewDataUrl:`data:image/png;base64,${qa.file.dataBase64}`}]);
      });
      await trigger.click(); await expect.poll(() => page.locator('dialog img').evaluate(img=>img.naturalWidth)).toBe(2400);
      assert.deepEqual(errors,[]);
      console.log(`${engine.name()}: 2400×1400 original preserved; 192px thumbnail stays lightweight; full-size/zoom, errors, focus and generated images pass.`);
    } finally { await browser.close(); }
  }
} finally { await new Promise(resolve => server.close(resolve)); }
