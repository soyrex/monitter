import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { fileURLToPath } from 'node:url';
import { chromium, webkit, expect } from '@playwright/test';
import { createServer } from 'node:http';
const result = await build({configFile:false,plugins:[svelte()],resolve:{alias:[{find:'$lib',replacement:`${process.cwd()}/src/lib`},{find:/^@lucide\/svelte(?:\/.*)?$/,replacement:fileURLToPath(new URL('./fixtures/lucide-stub.svelte',import.meta.url))}]},build:{write:false,minify:false,rollupOptions:{input:'scripts/fixtures/browser-pane-geometry-entry.js'}}});
const files=new Map(result.output.map(file=>[`/${file.fileName}`,file.type==='asset'?file.source:file.code]));
const entry=result.output.find(file=>file.type==='chunk'&&file.isEntry);
const css=result.output.filter(file=>file.type==='asset'&&file.fileName.endsWith('.css'));
files.set('/',`<!doctype html><style>body{margin:0}</style><div id="app"></div>${css.map(file=>`<link rel="stylesheet" href="/${file.fileName}">`).join('')}<script type="module" src="/${entry.fileName}"></script>`);
const server = createServer((req,res) => { const body=files.get(req.url); res.writeHead(body ? 200 : 404, {'content-type':req.url==='/'?'text/html':req.url.endsWith('.css')?'text/css':'text/javascript'}); res.end(body); });
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
try {
  for (const browserType of [chromium, webkit]) {
    const browser=await browserType.launch({headless:true});
    try {
      const page=await browser.newPage(); const errors=[]; page.on('pageerror',e=>errors.push(e.message));
      await page.goto(`http://127.0.0.1:${server.address().port}`);
      const finalRect = () => page.evaluate(() => { const rect=document.querySelector('.browser-native-viewport').getBoundingClientRect(); return {x:Math.round(rect.x),y:Math.round(rect.y),width:Math.round(rect.width),height:Math.round(rect.height),visible:true}; });
      const matches=async()=>expect.poll(()=>page.evaluate(()=>window.paneQA.reports.at(-1))).toEqual(await finalRect());
      await matches();
      const separator=page.getByRole('separator',{name:'Resize panes'});
      const box=await separator.boundingBox();
      await page.mouse.move(box.x+box.width/2,box.y+box.height/2);
      await page.mouse.down();
      await expect(page.getByLabel('Browser resize placeholder')).toBeVisible();
      await expect(page.getByText('https://example.test — browser',{exact:true})).toBeVisible();
      await expect.poll(()=>page.evaluate(()=>window.paneQA.reports.at(-1)?.visible)).toBe(false);
      const hideIndex=await page.evaluate(()=>window.paneQA.reports.length-1);
      await page.mouse.move(box.x-180,box.y+box.height/2,{steps:10});
      await page.waitForTimeout(80);
      expect(await page.evaluate(index=>window.paneQA.reports.slice(index).every(rect=>!rect.visible),hideIndex)).toBe(true);
      await page.mouse.up();
      await expect(page.getByLabel('Browser resize placeholder')).toHaveCount(0);
      await matches();
      // A sidebar drag also moves browser panes and must share the guard.
      const sidebar=page.getByRole('separator',{name:'Resize fixture sidebar'});
      await sidebar.dispatchEvent('pointerdown',{pointerId:31,button:0});
      await expect(page.getByLabel('Browser resize placeholder')).toBeVisible();
      await page.evaluate(()=>window.dispatchEvent(new PointerEvent('pointercancel',{pointerId:31})));
      await expect(page.getByLabel('Browser resize placeholder')).toHaveCount(0);await matches();
      // Blur and window-size bursts have a bounded final-frame reveal.
      await sidebar.dispatchEvent('pointerdown',{pointerId:32,button:0});
      await page.evaluate(()=>window.dispatchEvent(new Event('blur')));
      await expect(page.getByLabel('Browser resize placeholder')).toHaveCount(0);await matches();
      await page.evaluate(()=>window.dispatchEvent(new Event('resize')));
      await expect(page.getByLabel('Browser resize placeholder')).toBeVisible();
      await expect(page.getByLabel('Browser resize placeholder')).toHaveCount(0);await matches();
      await page.evaluate(()=>window.paneQA.resize(.65)); await page.waitForTimeout(300); await matches();
      await page.evaluate(()=>window.paneQA.move()); await page.waitForTimeout(300); await matches();
      await page.evaluate(()=>window.paneQA.closeOther()); await page.waitForTimeout(300); await matches();
      await page.setViewportSize({width:1100,height:760}); await page.evaluate(()=>window.dispatchEvent(new Event('resize'))); await expect(page.getByLabel('Browser resize placeholder')).toBeVisible(); await expect(page.getByLabel('Browser resize placeholder')).toHaveCount(0); await matches();
      const count=await page.evaluate(()=>window.paneQA.reports.length); await page.waitForTimeout(300); expect(await page.evaluate(()=>window.paneQA.reports.length)).toBe(count);
      expect(errors).toEqual([]); console.log(`${browserType.name()}: real PaneGrid neighbor resize, rearrange, close, window resize and idle measurement passed`);
    } finally {await browser.close();}
  }
} finally {await new Promise(resolve=>server.close(resolve));}
