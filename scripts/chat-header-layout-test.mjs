import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';

const source = readFileSync(new URL('../src/lib/components/AppSurface.svelte', import.meta.url), 'utf8');
const channelStart = source.indexOf('{:else if pane === "channel" && activeChannel}<section');
const channel = source.slice(channelStart, source.indexOf('<MessagePane', channelStart));
assert.ok(channel.includes('conversation-head task-heading pane-task-header'), 'Group chats use the shared full-width chat header');
assert.ok(channel.includes('avatar task-header-avatar'), 'Group chat header has an icon in the same avatar slot');
assert.ok(channel.includes('paneExpandControl()') && channel.includes('rightSidebarControl()'), 'Group chat header keeps both pane controls');
assert.ok(source.includes('class="avatar task-header-avatar" aria-label={selectedAgent?.name'), 'Direct chat uses the selected agent avatar');
assert.ok(source.includes("{:else if !activeWorkspaceKey.startsWith('agent:')}\n        {@render workspaceContext()}"), 'Agent workspace tabs do not duplicate the avatar');
assert.ok(source.includes('.conversation-head.pane-task-header { padding:15px; gap:10px; }'));
assert.ok(source.includes('.pane-task-header > h1 { font-size:calc(13.2px * var(--interface-font-ratio, 1)); }'));
const headerCss = source.slice(source.indexOf('  .conversation-head {')).split('}')[0];
assert.ok(headerCss.includes('background: var(--paper);'));
assert.ok(headerCss.includes('backdrop-filter: none;'));
console.log('Direct and group chats share a solid, compact, padded header; avatar is not duplicated in the tab bar.');
