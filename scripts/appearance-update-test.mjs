import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';
import { appThemePreset, applyThemeContrast, mixThemeColour } from '../src/lib/app-theme.ts';
import { contrastForeground } from '../src/lib/accent-contrast.ts';

const source = fs.readFileSync(new URL('../src/lib/appearance-key.ts', import.meta.url), 'utf8');
const js = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText;
const keyModule = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(js)}#appearance-key-test`);
const { appearanceKey, browserChromeKey } = keyModule;

const settings = {
  userName: 'Alex', shortcutMode: 'standard', showTabCloseButtons: true, autoHideTabs: false,
  tabStyle: 'classic', interfaceDensity: 'normal', tintUserMessages: true,
  compressToolCalls: false, terminalFontSize: 14, chatFontSize: 13, interfaceFontSize: 14,
  chatLineHeight: 1.65, terminalLineHeight: 1, terminalFont: 'IBM Plex Mono',
  chatFont: 'IBM Plex Sans', interfaceFont: 'IBM Plex Sans', windowSurface: 'opaque',
  windowTransparency: 18, showActivePaneBorder: true, dimInactivePanes: false,
  inactivePaneOpacity: 0.55, focusFollowsMouse: false, accent: '#3f9d6a', theme: 'system',
  interfaceScale: 125, showToolActivity: true, showReasoningSummaries: true,
  sendWithEnter: true, sidebarView: 'standard', busyMessageMode: 'queue',
};
const theme = { light: 'monitter', dark: 'monitter', accent: '#3f9d6a', contrast: 0 };
const appearanceFields = [
  'theme', 'windowSurface', 'interfaceDensity', 'interfaceFontSize', 'chatFontSize',
  'terminalFontSize', 'chatLineHeight', 'terminalLineHeight', 'windowTransparency',
  'interfaceFont', 'chatFont', 'terminalFont',
];
const changed = [];
for (const field of appearanceFields) {
  const next = { ...settings, [field]: field === 'theme' ? 'dark' : typeof settings[field] === 'number' ? settings[field] + 1 : `${settings[field]} changed` };
  assert.notEqual(appearanceKey(settings, theme, 125, 20, true), appearanceKey(next, theme, 125, 20, true), `${field} must invalidate appearance`);
  changed.push(field);
}
for (const field of ['light', 'dark', 'accent', 'contrast']) {
  const next = { ...theme, [field]: field === 'contrast' ? 'high' : `${theme[field]}-changed` };
  assert.notEqual(appearanceKey(settings, theme, 125, 20, true), appearanceKey(settings, next, 125, 20, true), `theme.${field} must invalidate appearance`);
}
assert.notEqual(appearanceKey(settings, theme, 125, 20, true), appearanceKey(settings, theme, 126, 20, true));
assert.notEqual(appearanceKey(settings, theme, 125, 20, true), appearanceKey(settings, theme, 125, 21, true));
assert.notEqual(appearanceKey(settings, theme, 125, 20, true), appearanceKey(settings, theme, 125, 20, false));

for (const field of ['userName', 'shortcutMode', 'showToolActivity', 'sendWithEnter', 'sidebarView', 'busyMessageMode']) {
  assert.equal(appearanceKey(settings, theme, 125, 20, true), appearanceKey({ ...settings, [field]: 'unrelated' }, theme, 125, 20, true), `${field} must not invalidate appearance`);
}

const chromeSettings = { ...settings, theme: 'dark' };
const chromeBase = browserChromeKey(chromeSettings, theme, 20, false);
assert.notEqual(chromeBase, browserChromeKey({ ...settings, theme: 'light' }, theme, 20, false));
assert.notEqual(chromeBase, browserChromeKey(settings, { ...theme, accent: '#cc5500' }, 20, false));
assert.notEqual(chromeBase, browserChromeKey(settings, theme, 21, false));
assert.equal(browserChromeKey(chromeSettings, theme, 20, false), browserChromeKey(chromeSettings, theme, 20, true), 'system colour should be ignored outside system mode');
const systemChrome = { ...settings, theme: 'system' };
assert.notEqual(browserChromeKey(systemChrome, theme, 20, false), browserChromeKey(systemChrome, theme, 20, true), 'system colour should update system chrome');
console.log(`appearance keys passed (${changed.length} settings, 4 theme fields, scale/tint/native, unrelated-field and system-chrome checks)`);

// Exercise the production renderer bodies without importing the Svelte component.
// This keeps the test deterministic while still testing the actual cache guards.
const surface = fs.readFileSync(new URL('../src/lib/components/AppSurface.svelte', import.meta.url), 'utf8');
const bodyStart = surface.indexOf('  function rgb(');
const bodyEnd = surface.indexOf('  $effect(() => {', bodyStart);
assert.ok(bodyStart >= 0 && bodyEnd > bodyStart, 'appearance renderer block must remain extractable');
const rendererSource = ts.transpileModule(surface.slice(bodyStart, bodyEnd), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None },
}).outputText;

const writes = { style: 0, remove: 0, dataset: 0, storage: 0, dispatch: 0, zoom: 0 };
const style = {
  backgroundColor: '',
  setProperty(name, value) { writes.style++; this[name] = value; },
  getPropertyValue(name) { return this[name] ?? ''; },
  removeProperty(name) { writes.remove++; delete this[name]; },
};
const root = { style, dataset: new Proxy({}, { set(target, name, value) { writes.dataset++; target[name] = value; return true; } }) };
const body = { style: { backgroundColor: '' } };
const localStorage = { setItem() { writes.storage++; } };
globalThis.document = { documentElement: root, body, querySelector: () => ({ setAttribute() { writes.style++; } }) };
globalThis.Event = class Event { constructor(type) { this.type = type; } };
let systemDark = false;
globalThis.window = {
  localStorage,
  matchMedia: () => ({ get matches() { return systemDark; } }),
  dispatchEvent() { writes.dispatch++; },
};
let zoomReject = false;
const getCurrentWebview = () => ({ setZoom() { writes.zoom++; return zoomReject ? Promise.reject(new Error('mock zoom failure')) : Promise.resolve(); } });
const text = reason => String(reason);
const rendererFactory = (tint, native) => new Function('appearanceKey', 'browserChromeKey', 'applyThemeContrast', 'appThemePreset', 'mixThemeColour', 'contrastForeground', 'getCurrentWebview', 'text', 'activeInterfaceScale', 'nativeRuntime', '$appTheme', '$surfaceTint', 'appliedScale', 'error', rendererSource + '\nreturn { applyAppearance, syncBrowserChrome };')(appearanceKey, browserChromeKey, applyThemeContrast, appThemePreset, mixThemeColour, contrastForeground, getCurrentWebview, text, 125, native, theme, tint, 0, '');
const renderer = rendererFactory(20, false);
const before = { ...writes };
renderer.applyAppearance(settings, theme, 125);
const afterFirst = { ...writes };
for (let i = 0; i < 1000; i++) renderer.applyAppearance({ ...settings }, { ...theme }, 125);
assert.deepEqual(writes, afterFirst, 'identical fresh snapshot objects must not repeat DOM/storage writes');
const afterRepeats = { ...writes };
assert.ok(afterFirst.style > before.style && afterFirst.storage > before.storage, 'first appearance must write DOM and storage');
renderer.applyAppearance({ ...settings, chatFont: 'Changed Font' }, theme, 125);
assert.ok(writes.style > afterFirst.style, 'font change must reapply appearance');
const afterFont = { ...writes };
renderer.applyAppearance(settings, { ...theme, light: 'nord', dark: 'dracula' }, 125);
assert.ok(writes.style > afterFont.style, 'theme palette change must reapply appearance');
const afterTheme = { ...writes };
renderer.applyAppearance(settings, { ...theme, accent: '#cc5500', contrast: 1 }, 125);
assert.ok(writes.style > afterTheme.style, 'accent/contrast change must reapply appearance');
renderer.applyAppearance(settings, theme, 126);
assert.ok(writes.dispatch > afterFirst.dispatch, 'scale change must dispatch scale event');
const systemSettings = { ...settings, theme: 'system' };
renderer.syncBrowserChrome(systemSettings, theme, 20);
const chromeWrites = { ...writes };
renderer.syncBrowserChrome(systemSettings, theme, 20);
assert.deepEqual(writes, chromeWrites, 'direct repeated browser chrome sync must be cached');
systemDark = true;
renderer.syncBrowserChrome(systemSettings, theme, 20);
assert.ok(writes.storage > chromeWrites.storage, 'system media change must refresh browser chrome storage');
const afterSystem = { ...writes };
renderer.syncBrowserChrome({ ...settings, theme: 'dark' }, theme, 20);
const afterDark = { ...writes };
systemDark = false;
renderer.syncBrowserChrome({ ...settings, theme: 'dark' }, theme, 20);
assert.deepEqual(writes, afterDark, 'explicit dark theme must ignore system media changes');
assert.ok(afterDark.storage >= afterSystem.storage, 'explicit dark sync remains observable');
zoomReject = true;
const nativeRenderer = rendererFactory(20, true);
nativeRenderer.applyAppearance(settings, theme, 125);
await Promise.resolve();
await Promise.resolve();
assert.equal(writes.zoom, 1, 'native first appearance attempts zoom');
zoomReject = false;
nativeRenderer.applyAppearance({ ...settings }, { ...theme }, 125);
assert.equal(writes.zoom, 2, 'failed native zoom clears cache so same-key retry is attempted');
nativeRenderer.applyAppearance({ ...settings }, { ...theme }, 125);
assert.equal(writes.zoom, 2, 'successful native zoom retry remains cached');
console.log(`production appearance renderer guard passed (first=${JSON.stringify(afterFirst)}, after1000Repeats=${JSON.stringify(afterRepeats)}, final=${JSON.stringify(writes)})`);
