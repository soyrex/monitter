// Opt-in protocol smoke test, NOT a model-turn test or an installer.
// node scripts/acp-handshake.mjs /absolute/path/to/agent [ACP arguments...]
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, isAbsolute } from 'node:path';

const [command,...args] = process.argv.slice(2);
if (!command || !isAbsolute(command)) throw new Error('Supply an explicit absolute executable path and its ACP arguments.');
const cwd = await mkdtemp(join(tmpdir(),'monitter-acp-handshake-'));
let child;
try {
  child = spawn(command,args,{cwd,stdio:['pipe','pipe','pipe'],detached:process.platform!=='win32'});
  const result = await new Promise((resolve,reject) => {
    let pending = Buffer.alloc(0), total = 0;
    const timeout = setTimeout(() => finish(new Error('ACP initialize timed out (20 seconds).')),20000);
    let finished = false;
    function finish(error,value) {
      if (finished) return;
      finished = true;
      clearTimeout(timeout);
      if (error) reject(error); else resolve(value);
    }
    child.on('error', () => finish(new Error('Could not launch the configured ACP executable.')));
    child.on('exit', () => finish(new Error('Agent exited before completing ACP initialize.')));
    child.stdin.on('error', () => finish(new Error('Agent closed the ACP input pipe.')));
    child.stderr.on('data', () => {}); // Drain, never echo auth/config diagnostics.
    child.stdout.on('data', chunk => {
      if (finished) return;
      total += chunk.length;
      if (total > 2*1024*1024) return finish(new Error('ACP initialize output exceeded 2 MiB.'));
      pending = Buffer.concat([pending,chunk]);
      let newline;
      while ((newline=pending.indexOf(10))>=0) {
        const line=pending.subarray(0,newline);pending=pending.subarray(newline+1);
        if (!line.toString('utf8').trim()) continue;
        let frame;
        try { frame=JSON.parse(line.toString('utf8')); }
        catch { return finish(new Error('Agent stdout was not ACP JSONL (payload withheld).')); }
        if (frame.jsonrpc!=='2.0') return finish(new Error('Agent did not use JSON-RPC 2.0.'));
        if (frame.method && Object.hasOwn(frame,'id')) {
          const reply=frame.method==='session/request_permission'
            ? {result:{outcome:{outcome:'cancelled'}}}
            : {error:{code:-32601,message:'Initialize-only probe; client operations are unavailable.'}};
          child.stdin.write(JSON.stringify({jsonrpc:'2.0',id:frame.id,...reply})+'\n');
        } else if (frame.id===1) {
          if (frame.error) return finish(new Error(`ACP initialize returned error code ${Number.isInteger(frame.error.code)?frame.error.code:'unknown'} (details withheld).`));
          if (frame.result?.protocolVersion!==1) return finish(new Error('Agent did not negotiate ACP v1.'));
          finish(null,frame.result);
        }
      }
    });
    child.stdin.write(JSON.stringify({jsonrpc:'2.0',id:1,method:'initialize',params:{protocolVersion:1,clientCapabilities:{},clientInfo:{name:'monitter-handshake-test',version:'0.1.0'}}})+'\n');
  });
  const safe = value => typeof value==='string'?value.replace(/[\x00-\x1f\x7f]/g,'').slice(0,160):null;
  const caps=result.agentCapabilities??{};
  console.log(JSON.stringify({verified:'initialize only; no authentication or model turn tested',protocolVersion:result.protocolVersion,agent:{name:safe(result.agentInfo?.name),version:safe(result.agentInfo?.version)},capabilities:{loadSession:caps.loadSession===true,resume:!!caps.sessionCapabilities?.resume,image:caps.promptCapabilities?.image===true,audio:caps.promptCapabilities?.audio===true,embeddedContext:caps.promptCapabilities?.embeddedContext===true},authMethodCount:Array.isArray(result.authMethods)?result.authMethods.length:0},null,2));
} finally {
  if (child?.pid) {
    child.stdin.destroy();
    try { if (process.platform==='win32') child.kill('SIGTERM'); else process.kill(-child.pid,'SIGTERM'); } catch {}
    await Promise.race([new Promise(resolve => child.once('exit',resolve)),new Promise(resolve => setTimeout(resolve,300))]);
    try { if (process.platform==='win32') child.kill('SIGKILL'); else process.kill(-child.pid,'SIGKILL'); } catch {}
  }
  await rm(cwd,{recursive:true,force:true});
}
