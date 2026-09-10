import {spawn} from 'node:child_process';
import {mkdir,writeFile} from 'node:fs/promises';
import {resolve} from 'node:path';
import assert from 'node:assert/strict';

// Explicit invocation uses the chosen real harness and its existing account.
const [provider,location='local',model] = process.argv.slice(2);
assert(['codex','claude','opencode','hermes'].includes(provider),'Choose an implemented provider');
assert(['local','mira'].includes(location),'Choose local or mira');
const remote=location==='mira';
const root=resolve('verification',`provider-${provider}-${location}-${Date.now()}`);
await mkdir(root,{recursive:true});
const localCwd=resolve('verification/provider-workspace');
await mkdir(localCwd,{recursive:true});
const cwd=remote?'/home/alex/.local/share/monitter/smoke-workspace':localCwd;
const host={id:location,name:location,kind:remote?'ssh':'local',address:remote?'mira':'',user:'',port:0,identityFile:'',defaultCwd:cwd,
  codexPath:remote?'/home/alex/.npm-global/bin/codex':'/Users/alex/.local/bin/codex',
  claudePath:remote?'/home/alex/.local/bin/claude':'/Users/alex/.local/bin/claude',
  opencodePath:remote?'':'/Users/alex/.opencode/bin/opencode',
  hermesPath:remote?'/home/alex/.local/bin/hermes':'/Users/alex/.local/bin/hermes'};
const marker=`MONITTER_${provider.toUpperCase()}_${Date.now()}`;
const args=['--state-dir',root,'--cwd',cwd,'--host-json',JSON.stringify(host),'--provider',provider,
  '--prompt',`Remember this marker for my next message: ${marker}. Reply with exactly ${marker}. Do not use tools or change files.`,
  '--second-prompt','Reply with exactly the marker I asked you to remember in my previous message. Do not use tools or change files.'];
if(model)args.push('--model',model);
const child=spawn(resolve('src-tauri/target/debug/monitter-smoke'),args,{stdio:['ignore','pipe','pipe']});
let stdout='',stderr='';
child.stdout.on('data',b=>stdout+=b);child.stderr.on('data',b=>stderr+=b);
const timer=setTimeout(()=>child.kill('SIGTERM'),420000);
let exitCode;
try{exitCode=await new Promise((resolveExit,reject)=>{child.once('error',reject);child.once('exit',resolveExit);});}
finally{clearTimeout(timer);}
await writeFile(`${root}/stdout.txt`,stdout,{mode:0o600});
await writeFile(`${root}/stderr.txt`,stderr,{mode:0o600});
let result;
try{result=JSON.parse(stdout.trim());}catch{throw Error(`No structured result; inspect ${root} (exit ${exitCode})`);}
const evidence={provider,location,marker,exitCode,result,finishedAt:new Date().toISOString()};
await writeFile(`${root}/evidence.json`,JSON.stringify(evidence,null,2)+'\n',{mode:0o600});
console.log(JSON.stringify({...evidence,evidencePath:`${root}/evidence.json`},null,2));
assert.equal(exitCode,0);assert(result.ok);assert(result.persisted);assert(result.nativeSessionId);
assert(result.outputCount>=2);assert(result.lastAssistantText.includes(marker),'Resumed conversation lost its marker');
