import { writable } from 'svelte/store';
import { getBridge } from '$lib/bridge';
import type { TerminalRead, TerminalSession } from '$lib/types';
import '@xterm/xterm/css/xterm.css';

type Xterm = import('@xterm/xterm').Terminal;
type FitAddon = import('@xterm/addon-fit').FitAddon;
type Runtime = { session: TerminalSession; terminal?: Xterm; fit?: FitAddon; host?: HTMLElement; desiredHost?: HTMLElement; active?: boolean; mountGeneration: number; initPromise?: Promise<void>; afterSeq: number; decoder: TextDecoder; poll?: ReturnType<typeof setInterval>; polling?: boolean; draining?: boolean; writeChain: Promise<void>; resizeTimer?: ReturnType<typeof setTimeout>; observer?: ResizeObserver; themeObserver?: MutationObserver; error?: string; errorSource?: 'read'|'input'|'resize'|'init'|'close'; closing?: Promise<void> };
const runtimes = new Map<string, Runtime>();
export const terminalSessions = writable<Record<string, TerminalSession>>({});
export const terminalErrors = writable<Record<string, string>>({});

function publish() {
  const sessions: Record<string, TerminalSession> = {}, errors: Record<string, string> = {};
  for (const [id, runtime] of runtimes) { sessions[id] = runtime.session; if (runtime.error) errors[id] = runtime.error; }
  terminalSessions.set(sessions); terminalErrors.set(errors);
}
function applyRead(runtime: Runtime, read: TerminalRead) {
  const changed = runtime.session.status !== read.status || runtime.session.exitCode !== read.exitCode || runtime.errorSource==='read';
  if (read.truncated) {
    runtime.decoder = new TextDecoder();
    runtime.terminal?.write('\r\n\x1b[33m[Earlier terminal output was truncated.]\x1b[0m\r\n');
  }
  for (const chunk of read.chunks) runtime.terminal?.write(runtime.decoder.decode(new Uint8Array(chunk.data), { stream: true }));
  runtime.afterSeq = read.nextSeq; runtime.session = { ...runtime.session, status: read.status, exitCode: read.exitCode };
  if (read.status === 'exited') {
    // A bounded backend read can report exit with more queued chunks after this batch.
    runtime.draining = read.chunks.length > 0;
    if (!runtime.draining) { runtime.terminal?.write(runtime.decoder.decode()); stopPolling(runtime); }
  }
  if(runtime.errorSource==='read'){runtime.error=undefined;runtime.errorSource=undefined;} if(changed)publish();
}
async function poll(runtime: Runtime) {
  if (runtime.closing || runtime.polling || (runtime.session.status === 'exited' && !runtime.draining) || !runtime.terminal) return;
  runtime.polling = true;
  try { const result=await getBridge().readTerminal(runtime.session.id, runtime.afterSeq); if(runtimes.get(runtime.session.id)===runtime && !runtime.closing)applyRead(runtime,result); }
  catch (reason) { if(!runtime.error || runtime.errorSource==='read'){runtime.error = reason instanceof Error ? reason.message : String(reason);runtime.errorSource='read';publish();} }
  finally { runtime.polling = false; }
}
function startPolling(runtime: Runtime) { if (runtime.poll || (runtime.session.status === 'exited' && !runtime.draining) || !runtime.terminal) return; void poll(runtime); runtime.poll = setInterval(() => void poll(runtime), 200); }
function stopPolling(runtime: Runtime) { if (runtime.poll) clearInterval(runtime.poll); runtime.poll = undefined; }
export function registerTerminal(session: TerminalSession) { const current = runtimes.get(session.id); if (current) { current.session = session; publish(); return; } const runtime: Runtime = { session, draining:session.status==='exited', mountGeneration: 0, afterSeq: 0, decoder: new TextDecoder(), writeChain: Promise.resolve() }; runtimes.set(session.id, runtime); publish(); }

async function ensureTerminal(runtime: Runtime, host: HTMLElement, generation: number) {
  if (!runtime.terminal) {
    if (typeof window === 'undefined') return;
    if (!runtime.initPromise) runtime.initPromise = Promise.all([import('@xterm/xterm'), import('@xterm/addon-fit')]).then(([{ Terminal }, { FitAddon }]) => {
      if (runtime.closing || runtimes.get(runtime.session.id) !== runtime || !runtime.desiredHost) return;
      const mount = runtime.desiredHost;
      const terminal = new Terminal({ cursorBlink: true, scrollback: 10_000, convertEol: true, allowProposedApi: false, fontSize: 14, theme: theme() });
    const fit = new FitAddon(); terminal.loadAddon(fit);
    terminal.attachCustomKeyEventHandler(event => {
      const key = event.key.toLowerCase();
      // Let Monitter's command palette and display shortcuts bubble out of xterm.
      return !((event.metaKey && ['p', 'k', ',', '=', '+', '-'].includes(key)) || (event.ctrlKey && event.shiftKey && ['p', 'k'].includes(key)));
    });
    terminal.onData(data => queueWrite(runtime, data));
      runtime.terminal = terminal; runtime.fit = fit; terminal.open(mount);
    runtime.themeObserver = new MutationObserver(() => { if (runtime.terminal) runtime.terminal.options.theme = theme(); });
      runtime.themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'style'] });
    }).catch(reason => { runtime.terminal?.dispose(); runtime.terminal=undefined; runtime.fit=undefined; runtime.errorSource='init'; runtime.error = reason instanceof Error ? reason.message : String(reason); publish(); }).finally(()=>{runtime.initPromise=undefined;});
    await runtime.initPromise;
    if (runtime.mountGeneration !== generation || runtime.desiredHost !== host || !runtime.terminal) return;
  } else if (runtime.terminal.element && runtime.terminal.element.parentElement !== host) host.appendChild(runtime.terminal.element);
  runtime.host = host; refit(runtime); startPolling(runtime); if(runtime.active)runtime.terminal?.focus();
}
function theme() { const css = getComputedStyle(document.documentElement); return { background: css.getPropertyValue('--panel').trim() || '#101214', foreground: css.getPropertyValue('--ink').trim() || '#e5e7eb', cursor: css.getPropertyValue('--accent').trim() || '#77b58b', selectionBackground: css.getPropertyValue('--soft').trim() || '#2b3940' }; }
function queueWrite(runtime: Runtime, data: string) {
  runtime.writeChain = runtime.writeChain.then(async () => {
    if (runtime.closing || runtime.session.status !== 'running') return;
    await getBridge().writeTerminal(runtime.session.id,data);
    if(runtime.errorSource==='input'){runtime.error=undefined;runtime.errorSource=undefined;publish();}
  }).catch(reason=>{runtime.errorSource='input';runtime.error=reason instanceof Error?reason.message:String(reason);publish();});
}
function refit(runtime: Runtime) {
  if (!runtime.host || !runtime.fit || !runtime.terminal || runtime.host.clientWidth < 2 || runtime.host.clientHeight < 2) return;
  runtime.fit.fit();
  const cols=Math.max(10,Math.min(500,runtime.terminal.cols)),rows=Math.max(4,Math.min(300,runtime.terminal.rows));
  if(runtime.terminal.cols!==cols||runtime.terminal.rows!==rows)runtime.terminal.resize(cols,rows);
  void getBridge().resizeTerminal(runtime.session.id,cols,rows).then(()=>{
    if(runtime.errorSource==='resize'){runtime.error=undefined;runtime.errorSource=undefined;publish();}
  }).catch(reason=>{runtime.errorSource='resize';runtime.error=reason instanceof Error?reason.message:String(reason);publish();});
}
export async function mountTerminal(sessionId: string, host: HTMLElement) { const runtime = runtimes.get(sessionId); if (!runtime) return; runtime.desiredHost=host; const generation=++runtime.mountGeneration; await ensureTerminal(runtime, host, generation); if (runtime.mountGeneration!==generation || runtime.desiredHost!==host || runtime.host!==host) return; runtime.observer?.disconnect(); runtime.observer = new ResizeObserver(() => { if (runtime.resizeTimer) clearTimeout(runtime.resizeTimer); runtime.resizeTimer = setTimeout(() => refit(runtime), 80); }); runtime.observer.observe(host); }
export function setTerminalActive(sessionId: string, active: boolean) { const runtime = runtimes.get(sessionId); if(!runtime)return;runtime.active=active;if(active && runtime.host){refit(runtime);runtime.terminal?.focus();} }
export function unmountTerminal(sessionId: string, host: HTMLElement) { const runtime = runtimes.get(sessionId); if (!runtime || runtime.desiredHost !== host) return; runtime.mountGeneration++; runtime.desiredHost=undefined; if (runtime.host===host) { runtime.observer?.disconnect(); runtime.observer = undefined; if (runtime.terminal?.element?.parentElement === host) host.removeChild(runtime.terminal.element); runtime.host = undefined; } }
export async function closeTerminalSession(id: string): Promise<void> { const runtime = runtimes.get(id); if (!runtime) return; if (!runtime.closing) runtime.closing = getBridge().closeTerminal(id).then(() => { stopPolling(runtime); runtime.observer?.disconnect(); runtime.themeObserver?.disconnect(); if (runtime.resizeTimer) clearTimeout(runtime.resizeTimer); runtime.terminal?.dispose(); runtimes.delete(id); publish(); }).catch(reason => { runtime.closing = undefined; runtime.errorSource='close'; runtime.error = reason instanceof Error ? reason.message : String(reason); publish(); throw reason; }); return runtime.closing; }
export function terminalStatus(id: string) { return runtimes.get(id)?.session; }
