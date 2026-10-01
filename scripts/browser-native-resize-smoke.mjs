import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { createServer } from 'node:http';
import { mkdir, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';

// Builds a labelled, isolated native fixture; does not launch or alter the
// installed desktop. Keep this process running while using the test bundle.
const server=createServer((req,res)=>{res.writeHead(200,{'content-type':'text/html','cache-control':'no-store'});res.end('<!doctype html><style>body{margin:0;background:#145a32;color:white;font:24px sans-serif;padding:20px}</style><h1>Native browser fixture</h1><p>This is a local resize test page, not an agent conversation.</p>');});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
try {
  const url=`http://127.0.0.1:${server.address().port}/`;
  const root=process.cwd(),out=resolve('artifacts/browser-resize-native/assets');
  const result=await build({configFile:false,plugins:[svelte()],resolve:{alias:[{find:'$lib',replacement:`${root}/src/lib`},{find:/^@lucide\/svelte(?:\/.*)?$/,replacement:fileURLToPath(new URL('./fixtures/lucide-stub.svelte',import.meta.url))}]},define:{__BROWSER_SMOKE_URL__:JSON.stringify(url)},build:{write:false,minify:false,rollupOptions:{input:'scripts/fixtures/browser-pane-geometry-entry.js'}}});
  await mkdir(out,{recursive:true});
  for(const file of result.output){const path=resolve(out,file.fileName);await mkdir(resolve(path,'..'),{recursive:true});await writeFile(path,file.type==='asset'?file.source:file.code);}
  const entry=result.output.find(file=>file.type==='chunk'&&file.isEntry),css=result.output.filter(file=>file.type==='asset'&&file.fileName.endsWith('.css'));
  await writeFile(resolve(out,'index.html'),`<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1"><style>body{margin:0;background:#eee}</style><div id="app"></div>${css.map(file=>`<link rel="stylesheet" href="/${file.fileName}">`).join('')}<script type="module" src="/${entry.fileName}"></script>`);
  await writeFile(resolve('artifacts/browser-resize-native/tauri.conf.json'),JSON.stringify({identifier:'com.monitter.browser-resize-smoke',productName:'Monitter Resize Smoke',build:{beforeBuildCommand:'',frontendDist:'../artifacts/browser-resize-native/assets'},app:{windows:[{label:'main',title:'Monitter Resize Smoke',titleBarStyle:'Overlay',hiddenTitle:true,width:1440,height:940}]},bundle:{targets:['app']}},null,2));
  console.log(`Native resize fixture ready: ${url}\nBuild with CARGO_BUILD_JOBS=2 npm run tauri -- build --debug --bundles app --config artifacts/browser-resize-native/tauri.conf.json\nRun the isolated bundle with MONITTER_BROWSER_SMOKE_DISABLE_LAN=1. Never replace the installed app with this test fixture.`);
  await new Promise(resolve=>{process.once('SIGINT',resolve);process.once('SIGTERM',resolve);});
} finally {await new Promise(resolve=>server.close(resolve));}
