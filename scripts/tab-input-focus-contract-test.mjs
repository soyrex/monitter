import { readFile } from 'node:fs/promises';
import { strict as assert } from 'node:assert';

const source = await readFile(new URL('../src/lib/components/AppSurface.svelte', import.meta.url), 'utf8');
const focusStart = source.indexOf('function focusSelectedTabInput()');
const focusBody = source.slice(focusStart, source.indexOf('\n  function selectTabPicker', focusStart));
assert.ok(focusStart > 0, 'Tab input focus helper must exist');
assert.match(focusBody, /tick\(\)\.then/, 'Focus waits for the selected tab DOM');
assert.match(focusBody, /pane-leaf\[data-pane-id=/, 'Focus stays within the selected pane');
assert.match(focusBody, /xterm-helper-textarea/, 'Terminal tabs focus their terminal input');
assert.ok(focusBody.includes('textarea[aria-label="Task message"]'), 'Chat and draft tabs focus their composer');
assert.ok(focusBody.includes('textarea[aria-label="Channel message"]'), 'Channel tabs focus their composer');
assert.match(focusBody, /dialog\[open\]/, 'Tab focus cannot steal focus from a modal');
assert.match(source.slice(source.indexOf('function selectTabPicker'), source.indexOf('const currentTabLabel')), /focusSelectedTabInput\(\)/, 'Pointer tab selection requests input focus');
assert.match(source.slice(source.indexOf('function selectRelativeTab'), source.indexOf('function currentVimTab')), /focusSelectedTabInput\(\)/, 'Relative keyboard tab selection requests input focus');
assert.match(source.slice(source.indexOf('function selectVimTab'), source.indexOf('async function executeWorkspaceVim')), /focusSelectedTabInput\(\)/, 'Indexed and sidebar tab selection requests input focus');
const sidebarSelection = source.slice(source.indexOf('async function openAgentWorkspaceTab'), source.indexOf('const sidebarOpenTaskIds'));
assert.ok(sidebarSelection.indexOf('railAgentId = null') < sidebarSelection.indexOf('focusExistingTab(tab)'), 'Compact sidebar picker closes before its tab requests input focus');
console.log('Tab input focus contract: chat, draft, channel and terminal selection focus locally without overriding dialogs.');
