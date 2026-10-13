export const SETTINGS_GROUPS = ['Workspace', 'Agents & integrations', 'Security & access'] as const;
export const SETTINGS_CATEGORIES = [
  { id: 'profile', label: 'Profile', detail: 'Your name for new chats', group: 'Workspace' },
  { id: 'appearance', label: 'Appearance', detail: 'Theme, colours and surfaces', group: 'Workspace' },
  { id: 'interface', label: 'Layout & panes', detail: 'Sizing, tabs, panes and motion', group: 'Workspace' },
  { id: 'typography', label: 'Fonts & text', detail: 'Fonts, sizes and line height', group: 'Workspace' },
  { id: 'behaviour', label: 'Keyboard & focus', detail: 'Shortcuts and pane focus', group: 'Workspace' },
  { id: 'conversation', label: 'Conversation', detail: 'Sending messages and activity', group: 'Workspace' },
  { id: 'agents', label: 'Agents', detail: 'Agents, hosts and permissions', group: 'Agents & integrations' },
  { id: 'collaboration', label: 'Collaboration', detail: 'Project boards and guidance', group: 'Agents & integrations' },
  { id: 'extensions', label: 'MCP & plugins', detail: 'Servers, skills and safety', group: 'Agents & integrations' },
  { id: 'approvals', label: 'Approval rules', detail: 'Saved always-approve rules', group: 'Security & access' },
  { id: 'environment', label: 'Environment & secrets', detail: 'Shared keys for local agents', group: 'Security & access' },
  { id: 'lan', label: 'LAN access', detail: 'Connect from a browser', group: 'Security & access' },
  { id: 'connection', label: 'Connection', detail: 'Choose your paired workspace', group: 'Security & access' },
  { id: 'remote', label: 'Remote control', detail: 'Pair this desktop with a phone', group: 'Security & access' },
] as const;
export type SettingsCategoryId = typeof SETTINGS_CATEGORIES[number]['id'];
export const SETTINGS_CATEGORY_LABELS = Object.fromEntries(SETTINGS_CATEGORIES.map(item => [item.id, item.label])) as Record<SettingsCategoryId, string>;
export function normalizeSettingsCategory(value: string, fallback: SettingsCategoryId = 'appearance'): SettingsCategoryId {
  const canonical = ({ directory: 'agents', general: 'profile', terminal: 'typography', secrets: 'environment' } as Record<string, string>)[value] ?? value;
  return SETTINGS_CATEGORIES.some(item => item.id === canonical) ? canonical as SettingsCategoryId : fallback;
}
