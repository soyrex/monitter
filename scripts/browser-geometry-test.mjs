import { build } from 'vite';
import { chromium, webkit, expect } from '@playwright/test';
import { createServer } from 'node:http';
const result = await build({ configFile: false, build: { write: false, minify: false, rollupOptions: { input: 'scripts/fixtures/browser-geometry-entry.js' } } });
const files = new Map(result.output.map(file => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]));
const entry = result.output.find(file => file.type === 'chunk' && file.isEntry);
files.set('/', `<!doctype html><style>body{margin:0}#outer{display:flex;width:800px;height:400px}#neighbor{width:200px;flex-shrink:0}#pane{flex:1;min-width:0}#viewport{width:300px;height:300px}</style><div id="outer"><div id="neighbor"></div><div id="pane"><div id="viewport"></div></div></div><script type="module" src="/${entry.fileName}"></script>`);
const server = createServer((req,res) => { const body=files.get(req.url); res.writeHead(body ? 200 : 404, {'content-type':req.url==='/'?'text/html':'text/javascript'}); res.end(body); });
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
try {
  for (const browserType of [chromium, webkit]) {
    const browser=await browserType.launch({headless:true});
    try {
      const page=await browser.newPage(); const errors=[]; page.on('pageerror',e=>errors.push(e.message));
      await page.goto(`http://127.0.0.1:${server.address().port}`);
      const finalRect = () => page.evaluate(() => { const rect=document.querySelector('#viewport').getBoundingClientRect(); return {x:Math.round(rect.x),y:Math.round(rect.y),width:Math.round(rect.width),height:Math.round(rect.height),visible:true}; });
      const matches = async () => expect.poll(()=>page.evaluate(()=>window.reports.at(-1))).toEqual(await finalRect());
      await matches();
      // Resize a sibling while the viewport dimensions remain fixed.
      await page.evaluate(()=>document.querySelector('#neighbor').style.width='280px'); await matches();
      // FLIP motion changes only position; there is no resize or transitionend.
      await page.evaluate(()=>{ const pane=document.querySelector('#pane'); pane.animate([{transform:'translate(-120px, 50px)'},{transform:'translate(0,0)'}],{duration:240,fill:'none'}); window.dispatchEvent(new Event('monitter:pane-geometry')); });
      await page.waitForTimeout(320); await matches();
      // Closing another pane shifts this viewport without resizing it.
      await page.evaluate(()=>document.querySelector('#neighbor').remove()); await matches();
      await page.evaluate(()=>{document.querySelector('#viewport').style.width='460px';document.querySelector('#viewport').style.height='220px';}); await matches();
      const beforeBurst=await page.evaluate(()=>window.reports.length);
      await page.evaluate(()=>{for(let i=0;i<30;i++)window.geometry.update(true,false,i);}); await matches();
      expect(await page.evaluate(()=>window.reports.length)).toBe(beforeBurst+1);
      expect(await page.evaluate(()=>window.reports.slice(-5).every(r=>r.visible))).toBe(true);
      const count=await page.evaluate(()=>window.reports.length); await page.waitForTimeout(300); expect(await page.evaluate(()=>window.reports.length)).toBe(count);
      await page.evaluate(()=>window.geometry.update(false,false,31)); expect(await page.evaluate(()=>window.reports.at(-1).visible)).toBe(false);
      await page.evaluate(()=>window.geometry.update(true,false,32)); await matches();
      await page.evaluate(()=>{
        const parent=document.createElement('div'); parent.id='moved-parent';parent.style.marginLeft='70px';
        document.body.append(parent);parent.append(document.querySelector('#viewport'));
        window.geometry.update(true,false,33);
      }); await matches();
      await page.evaluate(()=>{document.querySelector('#moved-parent').animate([{transform:'translate(80px,30px)'},{transform:'translate(0,0)'}],{duration:200});window.dispatchEvent(new Event('monitter:pane-geometry'));});
      await page.waitForTimeout(260);await matches();
      await page.evaluate(()=>window.geometry.destroy()); expect(await page.evaluate(()=>window.reports.at(-1).visible)).toBe(false);
      const stopped=await page.evaluate(()=>window.reports.length); await page.evaluate(()=>window.dispatchEvent(new Event('resize'))); await page.waitForTimeout(100); expect(await page.evaluate(()=>window.reports.length)).toBe(stopped);
      expect(errors).toEqual([]); console.log(`${browserType.name()}: sibling resize/close, WAAPI motion, pane resize, revision bursts, idle dedupe, hide and cleanup passed`);
    } finally {await browser.close();}
  }
} finally {await new Promise(resolve=>server.close(resolve));}
