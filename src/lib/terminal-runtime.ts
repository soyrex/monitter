import { writable } from 'svelte/store';
import { getBridge } from '$lib/bridge';
import { terminalPalette, terminalTheme, type TerminalThemeId } from '$lib/terminal-theme';
import type { TerminalRead, TerminalSession } from '$lib/types';
import '@xterm/xterm/css/xterm.css';

type Xterm = import('@xterm/xterm').Terminal;
type FitAddon = import('@xterm/addon-fit').FitAddon;
type Runtime = { session: TerminalSession; terminal?: Xterm; fit?: FitAddon; host?: HTMLElement; desiredHost?: HTMLElement; active?: boolean; mountGeneration: number; initPromise?: Promise<void>; afterSeq: number; decoder: TextDecoder; recentOutput: string; poll?: ReturnType<typeof setTimeout>; polling?: boolean; draining?: boolean; writeChain: Promise<void>; resizeTimer?: ReturnType<typeof setTimeout>; observer?: ResizeObserver; themeObserver?: MutationObserver; error?: string; errorSource?: 'read'|'input'|'resize'|'init'|'close'; closing?: Promise<void> };
const runtimes = new Map<string, Runtime>();
export const terminalSessions = writable<Record<string, TerminalSession>>({});
export const terminalErrors = writable<Record<string, string>>({});
let titleRefresh: ReturnType<typeof setTimeout> | undefined;
let titleRefreshing = false;
let selectedTerminalTheme: TerminalThemeId = 'monitter';
let documentVisible = typeof document === 'undefined' || document.visibilityState !== 'hidden';

function runtimeCanPoll(runtime: Runtime) {
  return documentVisible && !!runtime.active && !!runtime.host && !runtime.closing && !!runtime.terminal && (runtime.session.status !== 'exited' || !!runtime.draining);
}
function titleRefreshNeeded() {
  return documentVisible && [...runtimes.values()].some(runtime => runtime.active && runtime.host && !runtime.closing);
}
function documentVisibilityChanged() {
  documentVisible = document.visibilityState !== 'hidden';
  for (const runtime of runtimes.values()) {
    if (runtimeCanPoll(runtime)) startPolling(runtime);
    else stopPolling(runtime);
  }
  updateTitleRefresh();
  if (documentVisible) void refreshTerminalSessions();
}
if (typeof document !== 'undefined') document.addEventListener('visibilitychange', documentVisibilityChanged);

terminalTheme.subscribe(value => {
  selectedTerminalTheme = value;
  for (const runtime of runtimes.values()) {
    if (runtime.terminal) runtime.terminal.options.theme = theme();
  }
});

function publish() {
  const sessions: Record<string, TerminalSession> = {}, errors: Record<string, string> = {};
  for (const [id, runtime] of runtimes) { sessions[id] = runtime.session; if (runtime.error) errors[id] = runtime.error; }
  terminalSessions.set(sessions); terminalErrors.set(errors);
}
async function refreshTerminalSessions(activeOnly = true) {
  if (titleRefreshing || !runtimes.size) return;
  titleRefreshing = true;
  try {
    // The native monitor owns process observation. This is only a compact
    // session projection refresh, so hidden terminal tabs and sidebar rows
    // receive the same title state as the visible pane.
    // Read-driven projections already update titles. A slow compact refresh is
    // only useful for a terminal the user can currently see.
    const targets = new Map([...runtimes].filter(([, runtime]) => !runtime.closing && documentVisible && (!activeOnly || !!runtime.active && !!runtime.host)));
    if (!targets.size) return;
    let changed = false;
    for (const session of await getBridge().listTerminals()) {
      const runtime = targets.get(session.id);
      // Do not import terminals from another window, revive a closed runtime,
      // or replace the read-driven exit/drain state with a list snapshot.
      if (!runtime || runtimes.get(session.id) !== runtime || runtime.closing) continue;
      if (runtime.session.title !== session.title || runtime.session.autoTitle !== session.autoTitle || runtime.session.customTitle !== session.customTitle) {
        runtime.session = { ...runtime.session, title: session.title, autoTitle: session.autoTitle, customTitle: session.customTitle };
        changed = true;
      }
    }
    if (changed) publish();
  } catch { /* Visible read/input errors remain the actionable terminal errors. */ }
  finally { titleRefreshing = false; }
}
function updateTitleRefresh() {
  if (titleRefresh) { clearTimeout(titleRefresh); titleRefresh = undefined; }
  if (!titleRefreshNeeded()) return;
  // Foreground process names settle after five seconds on the backend; checking
  // active mounted terminals at the same cadence avoids a global one-second IPC.
  titleRefresh = setTimeout(() => { titleRefresh = undefined; void refreshTerminalSessions().finally(updateTitleRefresh); }, 5_000);
}
function applyRead(runtime: Runtime, read: TerminalRead) {
  const session = read.session ? { ...read.session, status: read.status, exitCode: read.exitCode } : { ...runtime.session, status: read.status, exitCode: read.exitCode };
  const changed = runtime.session.title !== session.title || runtime.session.autoTitle !== session.autoTitle || runtime.session.customTitle !== session.customTitle || runtime.session.status !== session.status || runtime.session.exitCode !== session.exitCode || runtime.errorSource==='read';
  if (read.truncated) {
    runtime.decoder = new TextDecoder();
    runtime.terminal?.write('\r\n\x1b[33m[Earlier terminal output was truncated.]\x1b[0m\r\n');
  }
  for (const chunk of read.chunks) { const text = runtime.decoder.decode(new Uint8Array(chunk.data), { stream: true }); runtime.terminal?.write(text); runtime.recentOutput = (runtime.recentOutput + text).slice(-12_000); }
  runtime.afterSeq = read.nextSeq;
  // Backend title state is authoritative. This also means a remounted pane
  // cannot reset a title which changed while its xterm was hidden.
  runtime.session = session;
  if (read.status === 'exited') {
    // A bounded backend read can report exit with more queued chunks after this batch.
    runtime.draining = read.chunks.length > 0;
    if (!runtime.draining) {
      runtime.terminal?.write(runtime.decoder.decode()); stopPolling(runtime);
      // Release only after the final output batch. Close failures remain visible and retryable.
      void closeTerminalSession(runtime.session.id).catch(() => {});
    }
  }
  if(runtime.errorSource==='read'){runtime.error=undefined;runtime.errorSource=undefined;} if(changed)publish();
}
async function poll(runtime: Runtime) {
  if (!runtimeCanPoll(runtime) || runtime.polling) return;
  runtime.polling = true;
  try { const result=await getBridge().readTerminal(runtime.session.id, runtime.afterSeq); if(runtimes.get(runtime.session.id)===runtime && !runtime.closing)applyRead(runtime,result); }
  catch (reason) { if(!runtime.error || runtime.errorSource==='read'){runtime.error = reason instanceof Error ? reason.message : String(reason);runtime.errorSource='read';publish();} }
  finally { runtime.polling = false; if (runtimeCanPoll(runtime)) schedulePoll(runtime); }
}
function schedulePoll(runtime: Runtime) {
  if (runtime.poll || !runtimeCanPoll(runtime)) return;
  runtime.poll = setTimeout(() => { runtime.poll = undefined; void poll(runtime); }, 200);
}
function startPolling(runtime: Runtime) { if (!runtimeCanPoll(runtime) || runtime.polling || runtime.poll) return; void poll(runtime); }
function stopPolling(runtime: Runtime) { if (runtime.poll) clearTimeout(runtime.poll); runtime.poll = undefined; }
export function registerTerminal(session: TerminalSession) { const current = runtimes.get(session.id); if (current) { current.session = session; publish(); return; } const runtime: Runtime = { session, draining:session.status==='exited', mountGeneration: 0, afterSeq: 0, decoder: new TextDecoder(), recentOutput: '', writeChain: Promise.resolve() }; runtimes.set(session.id, runtime); updateTitleRefresh(); publish(); }
export function recentTerminalOutput(id: string) {
  // Preserve only printable visible text for the naming prompt; terminal escape
  // sequences are control protocol, not pane content.
  return (runtimes.get(id)?.recentOutput ?? '').replace(/\x1B\[[0-?]*[ -\/]*[@-~]|[\x00-\x08\x0B\x0C\x0E-\x1F\x7F]/g, '').slice(-12_000);
}

async function ensureTerminal(runtime: Runtime, host: HTMLElement, generation: number) {
  if (!runtime.terminal) {
    if (typeof window === 'undefined') return;
    if (!runtime.initPromise) runtime.initPromise = Promise.all([import('@xterm/xterm'), import('@xterm/addon-fit')]).then(([{ Terminal }, { FitAddon }]) => {
      if (runtime.closing || runtimes.get(runtime.session.id) !== runtime || !runtime.desiredHost) return;
      const mount = runtime.desiredHost;
      const terminal = new Terminal({ cursorBlink: true, scrollback: 10_000, convertEol: true, allowProposedApi: false, fontSize: terminalFontSize(), fontFamily: terminalFont(), lineHeight: terminalLineHeight(), theme: theme() });
    const fit = new FitAddon(); terminal.loadAddon(fit);
    terminal.attachCustomKeyEventHandler(event => {
      const key = event.key.toLowerCase();
      // Let Monitter's command palette and display shortcuts bubble out of xterm.
      return !((event.metaKey && ['p', 'k', 'w', ',', '=', '+', '-'].includes(key)) || (event.ctrlKey && event.shiftKey && ['p', 'k'].includes(key)) || (event.ctrlKey && !/Mac|iPhone|iPad/.test(navigator.platform) && key === 'w'));
    });
    terminal.onData(data => queueWrite(runtime, data));
      runtime.terminal = terminal; runtime.fit = fit; terminal.open(mount);
    runtime.themeObserver = new MutationObserver(() => { if (runtime.terminal) {
      runtime.terminal.options.theme = theme();
      const font = terminalFont(), size = terminalFontSize(), lineHeight = terminalLineHeight();
      if (runtime.terminal.options.fontFamily !== font || runtime.terminal.options.fontSize !== size || runtime.terminal.options.lineHeight !== lineHeight) {
        runtime.terminal.options.fontSize = size;
        runtime.terminal.options.fontFamily = font;
        runtime.terminal.options.lineHeight = lineHeight;
        requestAnimationFrame(() => refit(runtime));
        void document.fonts.load(`${size}px ${font}`).then(() => { if (runtime.host && !runtime.closing) refit(runtime); });
      }
    } });
      runtime.themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'style'] });
    }).catch(reason => { runtime.terminal?.dispose(); runtime.terminal=undefined; runtime.fit=undefined; runtime.errorSource='init'; runtime.error = reason instanceof Error ? reason.message : String(reason); publish(); }).finally(()=>{runtime.initPromise=undefined;});
    await runtime.initPromise;
    if (runtime.mountGeneration !== generation || runtime.desiredHost !== host || !runtime.terminal) return;
  } else if (runtime.terminal.element && runtime.terminal.element.parentElement !== host) host.appendChild(runtime.terminal.element);
  runtime.host = host; refit(runtime); startPolling(runtime); updateTitleRefresh(); if(runtime.active)runtime.terminal?.focus();
}
function terminalFontSize() { return Math.max(8, Math.min(32, Number(getComputedStyle(document.documentElement).getPropertyValue('--terminal-font-size')) || 14)); }
function terminalFont() { return getComputedStyle(document.documentElement).getPropertyValue('--terminal-font').trim() || '"IBM Plex Mono", Menlo, monospace'; }
function terminalLineHeight() { return Math.max(1, Math.min(2.5, Number(getComputedStyle(document.documentElement).getPropertyValue('--terminal-line-height')) || 1)); }
function theme() {
  const selected = terminalPalette(selectedTerminalTheme);
  if (selectedTerminalTheme !== 'monitter') return selected;
  const css = getComputedStyle(document.documentElement);
  return {
    ...selected,
    background: css.getPropertyValue('--terminal-background').trim() || selected.background,
    foreground: css.getPropertyValue('--terminal-foreground').trim() || selected.foreground,
    cursor: css.getPropertyValue('--accent').trim() || selected.cursor,
  };
}
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
export async function mountTerminal(sessionId: string, host: HTMLElement) { const runtime = runtimes.get(sessionId); if (!runtime) return; runtime.desiredHost=host; const generation=++runtime.mountGeneration; await ensureTerminal(runtime, host, generation); if (runtime.mountGeneration!==generation || runtime.desiredHost!==host || runtime.host!==host) return; runtime.observer?.disconnect(); runtime.observer = new ResizeObserver(() => { if (runtime.resizeTimer) clearTimeout(runtime.resizeTimer); runtime.resizeTimer = setTimeout(() => refit(runtime), 80); }); runtime.observer.observe(host); startPolling(runtime); updateTitleRefresh(); }
export function setTerminalActive(sessionId: string, active: boolean) { const runtime = runtimes.get(sessionId); if(!runtime)return; runtime.active=active; if(active && runtime.host){refit(runtime);runtime.terminal?.focus();startPolling(runtime);void refreshTerminalSessions();} else stopPolling(runtime); updateTitleRefresh(); }
export function unmountTerminal(sessionId: string, host: HTMLElement) { const runtime = runtimes.get(sessionId); if (!runtime || runtime.desiredHost !== host) return; runtime.mountGeneration++; runtime.desiredHost=undefined; stopPolling(runtime); if (runtime.host===host) { runtime.observer?.disconnect(); runtime.observer = undefined; if (runtime.terminal?.element?.parentElement === host) host.removeChild(runtime.terminal.element); runtime.host = undefined; } updateTitleRefresh(); }
export async function closeTerminalSession(id: string): Promise<void> { const runtime = runtimes.get(id); if (!runtime) return; if (!runtime.closing) runtime.closing = getBridge().closeTerminal(id).then(() => { stopPolling(runtime); runtime.observer?.disconnect(); runtime.themeObserver?.disconnect(); if (runtime.resizeTimer) clearTimeout(runtime.resizeTimer); runtime.terminal?.dispose(); runtimes.delete(id); updateTitleRefresh(); publish(); }).catch(reason => { runtime.closing = undefined; runtime.errorSource='close'; runtime.error = reason instanceof Error ? reason.message : String(reason); publish(); throw reason; }); return runtime.closing; }
export function terminalStatus(id: string) { return runtimes.get(id)?.session; }

/** Narrow hooks for the isolated runtime regression harness; not used by the UI. */
export const terminalRuntimeTesting = {
  applyRead(id: string, read: TerminalRead) { const runtime = runtimes.get(id); if (!runtime) throw new Error(`Unknown terminal ${id}`); applyRead(runtime, read); },
  refresh: () => refreshTerminalSessions(false),
  discard(id: string) { const runtime = runtimes.get(id); if (runtime) stopPolling(runtime); runtimes.delete(id); updateTitleRefresh(); publish(); },
};
