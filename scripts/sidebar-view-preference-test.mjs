import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  loadSidebarViewPreference,
  saveSidebarViewPreference,
  sidebarViewStorageKey,
} from '../src/lib/sidebar-view-preference.ts';

const values = new Map();
const sessionValues = new Map();
globalThis.sessionStorage = {
  getItem: key => sessionValues.get(key) ?? null,
  setItem: (key, value) => sessionValues.set(key, value),
};
globalThis.localStorage = {
  getItem: key => values.get(key) ?? null,
  setItem: (key, value) => values.set(key, value),
};

assert.notEqual(sidebarViewStorageKey('desktop'), sidebarViewStorageKey('web'));
assert.notEqual(sidebarViewStorageKey('web'), sidebarViewStorageKey('mobile'));
assert.equal(loadSidebarViewPreference('desktop'), null);
assert.equal(saveSidebarViewPreference('desktop', 'activity'), true);
assert.equal(saveSidebarViewPreference('web', 'projects'), true);
assert.equal(saveSidebarViewPreference('mobile', 'standard'), true);
assert.equal(loadSidebarViewPreference('desktop'), 'activity');
assert.equal(loadSidebarViewPreference('web'), 'projects');
assert.equal(loadSidebarViewPreference('mobile'), 'standard');

values.set(sidebarViewStorageKey('web'), 'invalid');
assert.equal(loadSidebarViewPreference('web'), 'projects', 'this tab retains its choice when the shared local default changes');
sessionValues.delete(sidebarViewStorageKey('web'));
assert.equal(loadSidebarViewPreference('web'), null);
const originalSet = localStorage.setItem;
const originalSessionSet = sessionStorage.setItem;
localStorage.setItem = () => { throw new Error('Storage unavailable'); };
sessionStorage.setItem = () => { throw new Error('Storage unavailable'); };
assert.equal(saveSidebarViewPreference('desktop', 'projects'), false);
localStorage.setItem = originalSet;
sessionStorage.setItem = originalSessionSet;

const desktop = readFileSync(new URL('../src/lib/components/AppSurface.svelte', import.meta.url), 'utf8');
const mobile = readFileSync(new URL('../src/routes/mobile/+page.svelte', import.meta.url), 'utf8');
const contract = readFileSync(new URL('../docs/CONTRACT.md', import.meta.url), 'utf8');
assert.match(desktop, /sidebarViewState.view\s*=\s*view;[\s\S]{0,180}saveSidebarViewPreference\(sidebarViewClient, view\)/);
assert.doesNotMatch(desktop, /saveSettingsPatch\(\{\s*sidebarView\s*:/);
assert.match(desktop, /const sidebarViewClient:[^\n]+nativeRuntime \? 'desktop' : 'web'/);
assert.match(mobile, /saveSidebarViewPreference\('mobile', view\)/);
assert.match(contract, /stored locally and independently[\s\S]{0,100}native desktop, ordinary web and mobile clients/);

console.log('Sidebar view preference: desktop, web and mobile state are isolated and switching avoids backend settings writes.');
