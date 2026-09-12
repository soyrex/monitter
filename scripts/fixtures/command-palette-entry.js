import { mount } from 'svelte';
import CommandPalette from '../../src/lib/components/CommandPalette.svelte';

window.__PALETTE_QA__ = { selected: null, closed: false };
mount(CommandPalette, {
  target: document.getElementById('app'),
  props: {
    open: true, title: 'Switch to', placeholder: 'Find a channel, chat or agent…',
    items: [
      { id: 'settings:settings', label: 'Settings', group: 'Workspace' },
      { id: 'draft:1', label: 'New chat', group: 'Draft chats' },
      { id: 'task:1', label: 'Existing chat', group: 'Chats' },
      { id: 'terminal:1', label: 'Terminal: zsh', group: 'Terminals' },
      { id: 'channel:1', label: 'Everyone', group: 'Channels' },
      { id: 'agent:1', label: 'Agent', group: 'Agents' },
      { id: 'project:1', label: 'Project', group: 'Projects' },
    ],
    onselect: id => { window.__PALETTE_QA__.selected = id; },
    onclose: () => { window.__PALETTE_QA__.closed = true; },
  },
});
