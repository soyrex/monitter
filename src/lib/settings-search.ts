import { SETTINGS_CATEGORY_LABELS, type SettingsCategoryId } from './settings-navigation';
/** Public preference metadata only. Never include saved values, names, secrets or drafts. */
export interface SettingsSearchEntry {
  id: string; category: string; label: string; description: string; keywords?: readonly string[]; target?: string;
}
export const SETTINGS_SEARCH_ENTRIES: readonly SettingsSearchEntry[] = [
  { id: 'system-one', category: 'agents', label: 'System One', description: 'Default backend and task routing, with per-agent overrides.', keywords: ['SYS-1', 'Jev', 'Laya', 'TypeSafe', 'routing', 'backend'], target: 'system-one-heading' },
  { id: 'connection', category: 'connection', label: 'Workspace connection', description: 'Switch paired workspaces or reconnect this phone.', keywords: ['pairing', 'connection'], target: 'phone-connection-heading' },

{"id": "inactive-opacity", "category": "interface", "label": "Inactive pane opacity", "description": "Adjust opacity of inactive panes.", "target": "panes-heading"},
{"id": "dim-panes", "category": "interface", "label": "Dim inactive panes", "description": "Reduce visual weight of inactive panes.", "target": "panes-heading"},
{"id": "active-pane", "category": "interface", "label": "Highlight active pane", "description": "Draw an accent border around the active pane.", "target": "panes-heading"},
{"id": "pane-borders", "category": "interface", "label": "Pane borders", "description": "Control active pane border appearance.", "target": "panes-heading", "keywords": []},
{"id": "auto-hide-tabs", "category": "interface", "label": "Auto hide tabs", "description": "Reveal tabs by hovering over the pane top bar.", "target": "tabs-heading", "keywords": ["autohide"]},
{"id": "tab-close", "category": "interface", "label": "Show tab close buttons", "description": "Display close controls on tabs.", "target": "tabs-heading", "keywords": []},
{"id": "tab-style", "category": "interface", "label": "Tab style", "description": "Choose classic rounded or modern straight tabs.", "target": "tabs-heading", "keywords": []},
{"id": "scale", "category": "interface", "label": "Interface scale", "description": "Resize text and controls.", "target": "scale-heading", "keywords": ["zoom", "sizing"]},
  { id: 'speech', category: 'conversation', label: 'Phone voice transcription', description: 'Choose desktop Whisper or on-device speech recognition.', keywords: ['dictation', 'speech'], target: 'speech-heading' },
  { id: 'project-boards-enabled', category: 'collaboration', label: 'Enable project boards', description: 'Show project coordination channels and notes.', keywords: ['channels', 'collaboration'], target: 'project-board-heading' },
  { id: 'busy-steering', category: 'conversation', label: 'Default to steering busy agents', description: 'Choose steering or queuing messages during a running turn.', keywords: ['queue', 'follow up'], target: 'busy-heading' },

  {
    "id": "remote-control",
    "category": "remote",
    "label": "Pair a phone",
    "description": "Create a one-time pairing code from this desktop. The listener and remembered phones remain active while you use another Settings category or close Settings.",
    "target": "remote-control-heading"
  },
  {
    "id": "permissions",
    "category": "agents",
    "label": "Agent permissions",
    "description": "Permission modes are configured per agent. Each task keeps the permission mode captured when it was created, so changing an agent affects future tasks.",
    "target": "permissions-heading"
  },
  {
    "id": "saved-approvals",
    "category": "approvals",
    "label": "Saved approvals",
    "description": "These rules only cover the exact agent, host, folder and launch identity that created them. Revoking a rule affects future requests only; work already approved keeps running.",
    "target": "saved-approvals-heading"
  },
  {
    "id": "your-name",
    "category": "profile",
    "label": "Your name",
    "description": "Used by agents in new chats.",
    "target": "your-name-heading"
  },
  {
    "id": "theme",
    "category": "appearance",
    "label": "Appearance mode",
    "description": "Follow the system, or keep one of your two palettes active.",
    "target": "theme-heading"
  },
  {
    "id": "app-theme",
    "category": "appearance",
    "label": "App theme",
    "description": "Choose the palette Monitter uses in light and dark mode. System appearance switches between them automatically.",
    "target": "app-theme-heading"
  },
  {
    "id": "window-surface",
    "category": "appearance",
    "label": "Window surface",
    "description": "Choose how much shows through the connected desktop\u2019s native Monitter window. This does not make your phone or browser transparent.",
    "target": "window-surface-heading"
  },
  {
    "id": "contrast",
    "category": "appearance",
    "label": "Contrast",
    "description": "Push surfaces and text further apart while keeping the selected themes.",
    "target": "contrast-heading"
  },
  {
    "id": "accent",
    "category": "appearance",
    "label": "Accent colour",
    "description": "Override the selected themes with one accent in both modes.",
    "target": "accent-heading"
  },
  {
    "id": "borders",
    "category": "appearance",
    "label": "Borders & dividers",
    "description": "Adjust structural lines in both themes.",
    "target": "borders-heading"
  },
  {
    "id": "terminal-theme",
    "category": "appearance",
    "label": "Terminal colour theme",
    "description": "Choose a familiar ANSI palette for every terminal. Open terminals update immediately.",
    "target": "terminal-theme-heading"
  },
  {
    "id": "activity-animation",
    "category": "appearance",
    "label": "Working animation",
    "description": "Choose the accent effect shown while an agent is working.",
    "target": "activity-animation-heading"
  },
  {
    "id": "density",
    "category": "interface",
    "label": "Interface density",
    "description": "Adjust the height and spacing of tabs, sidebars and pane controls.",
    "target": "density-heading"
  },
  {
    "id": "tabs",
    "category": "interface",
    "label": "Tabs",
    "description": "Choose a classic rounded strip or a modern full-height straight-edge strip.",
    "target": "tabs-heading"
  },
  {
    "id": "panes",
    "category": "interface",
    "label": "Pane appearance",
    "description": "Keep the active workspace easy to identify.",
    "target": "panes-heading"
  },
  {
    "id": "motion",
    "category": "interface",
    "label": "Motion",
    "description": "System follows your device. On keeps Monitter animated even when Reduce Motion is enabled.",
    "target": "motion-heading"
  },
  {
    "id": "fonts",
    "category": "typography",
    "label": "Fonts",
    "description": "Choose from fonts installed on this desktop, or type a family name. Clear a field to restore its default.",
    "target": "fonts-heading"
  },
  {
    "id": "line-height",
    "category": "typography",
    "label": "Line height",
    "description": "Control the vertical space in chat messages and terminal output.",
    "target": "line-height-heading"
  },
  {
    "id": "shortcut",
    "category": "behaviour",
    "label": "Keyboard shortcuts",
    "description": "Choose how workspace commands and pane navigation behave.",
    "target": "shortcut-heading"
  },
  {
    "id": "focus",
    "category": "behaviour",
    "label": "Workspace focus",
    "description": "Set how split panes respond as you move through them.",
    "target": "focus-heading"
  },
  {
    "id": "project-board",
    "category": "collaboration",
    "label": "Project boards",
    "description": "Private coordination notes shown as #channels for each project.",
    "target": "project-board-heading"
  },
  {
    "id": "messages",
    "category": "conversation",
    "label": "Messages",
    "description": "Choose the keyboard shortcut that sends a task or channel message.",
    "target": "messages-heading"
  },
  {
    "id": "busy",
    "category": "conversation",
    "label": "Busy agents",
    "description": "Choose what happens when you send another message during a running turn.",
    "target": "busy-heading"
  },
  {
    "id": "voice-transcription",
    "category": "conversation",
    "label": "Voice transcription",
    "description": "Choose how voice recordings are transcribed on this desktop. The result is added to your editable message draft.",
    "target": "voice-transcription-heading"
  },
  {
    "id": "activity",
    "category": "conversation",
    "label": "Conversation activity",
    "description": "Control supporting information shown alongside messages.",
    "target": "activity-heading"
  },
  {
    "id": "transparency",
    "category": "appearance",
    "label": "Window transparency",
    "description": "Adjust desktop window transparency and glass blur.",
    "target": "window-surface-heading",
    "keywords": [
      "transparent",
      "opacity",
      "glass"
    ]
  },
  {
    "id": "reduce-motion",
    "category": "interface",
    "label": "Reduce motion",
    "description": "Follow system motion preferences or turn animations off.",
    "target": "motion-heading",
    "keywords": [
      "animation",
      "accessibility"
    ]
  },
  {
    "id": "guidance",
    "category": "collaboration",
    "label": "Default agent guidance",
    "description": "Edit default project-board coordination instructions.",
    "target": "project-board-heading",
    "keywords": [
      "instructions",
      "board guidance"
    ]
  },
  {
    "id": "focus-mouse",
    "category": "behaviour",
    "label": "Focus follows mouse",
    "description": "Activate a pane when the pointer enters it.",
    "target": "focus-heading",
    "keywords": [
      "hover"
    ]
  },
  {
    "id": "send-shortcut",
    "category": "conversation",
    "label": "Send message shortcut",
    "description": "Choose Enter or Command Enter to send messages.",
    "target": "messages-heading",
    "keywords": [
      "ctrl enter"
    ]
  },
  {
    "id": "tool-activity",
    "category": "conversation",
    "label": "Show tool activity",
    "description": "Show collapsible harness tool activity.",
    "target": "activity-heading",
    "keywords": []
  },
  {
    "id": "compress-tools",
    "category": "conversation",
    "label": "Compress tool calls",
    "description": "Combine tool activity within each turn.",
    "target": "activity-heading",
    "keywords": []
  },
  {
    "id": "reasoning",
    "category": "conversation",
    "label": "Show reasoning summaries",
    "description": "Display harness reasoning summaries.",
    "target": "activity-heading",
    "keywords": [
      "thinking"
    ]
  },
  {
    "id": "tint-messages",
    "category": "conversation",
    "label": "Tint my messages",
    "description": "Use accent colour for message bubbles.",
    "target": "activity-heading",
    "keywords": []
  },
  {
    "id": "mcp",
    "category": "extensions",
    "label": "MCP servers and plugins",
    "description": "Configure servers, plugins and skills.",
    "target": "",
    "keywords": [
      "extensions",
      "integrations"
    ]
  },
  {
    "id": "secrets",
    "category": "environment",
    "label": "Environment variables and secrets",
    "description": "Shared keys for local agents.",
    "target": "",
    "keywords": [
      "api keys"
    ]
  },
  {
    "id": "lan",
    "category": "lan",
    "label": "LAN browser access",
    "description": "Connect browsers on the local network.",
    "target": "",
    "keywords": [
      "network",
      "wifi"
    ]
  },
  {
    "id": "interface-font",
    "category": "typography",
    "label": "Interface font",
    "description": "Adjust interface font for text readability.",
    "target": "fonts-heading",
    "keywords": [
      "text",
      "typeface",
      "font"
    ]
  },
  {
    "id": "interface-font-size",
    "category": "typography",
    "label": "Interface font size",
    "description": "Adjust interface font size for text readability.",
    "target": "fonts-heading",
    "keywords": [
      "text",
      "typeface",
      "size"
    ]
  },
  {
    "id": "interface-font-weight",
    "category": "typography",
    "label": "Interface font weight",
    "description": "Adjust interface font weight for text readability.",
    "target": "fonts-heading",
    "keywords": [
      "text",
      "typeface",
      "font weight"
    ]
  },
  {
    "id": "chat-font",
    "category": "typography",
    "label": "Chat font",
    "description": "Adjust chat font for text readability.",
    "target": "fonts-heading",
    "keywords": [
      "text",
      "typeface",
      "font"
    ]
  },
  {
    "id": "chat-font-size",
    "category": "typography",
    "label": "Chat font size",
    "description": "Adjust chat font size for text readability.",
    "target": "fonts-heading",
    "keywords": [
      "text",
      "typeface",
      "size"
    ]
  },
  {
    "id": "chat-font-weight",
    "category": "typography",
    "label": "Chat font weight",
    "description": "Adjust chat font weight for text readability.",
    "target": "fonts-heading",
    "keywords": [
      "text",
      "typeface",
      "font weight"
    ]
  },
  {
    "id": "terminal-font",
    "category": "typography",
    "label": "Terminal font",
    "description": "Adjust terminal font for text readability.",
    "target": "fonts-heading",
    "keywords": [
      "text",
      "typeface",
      "font"
    ]
  },
  {
    "id": "terminal-font-size",
    "category": "typography",
    "label": "Terminal font size",
    "description": "Adjust terminal font size for text readability.",
    "target": "fonts-heading",
    "keywords": [
      "text",
      "typeface",
      "size"
    ]
  },
  {
    "id": "terminal-font-weight",
    "category": "typography",
    "label": "Terminal font weight",
    "description": "Adjust terminal font weight for text readability.",
    "target": "fonts-heading",
    "keywords": [
      "text",
      "typeface",
      "font weight"
    ]
  }
];

function normalize(value: string): string {
  return value.normalize('NFKD').replace(/[\u0300-\u036f]/g, '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();
}
function distance(left: string, right: string): number {
  let row = Array.from({ length: right.length + 1 }, (_, i) => i);
  for (let i = 1; i <= left.length; i++) {
    const next = [i];
    for (let j = 1; j <= right.length; j++) next[j] = Math.min(next[j - 1] + 1, row[j] + 1, row[j - 1] + Number(left[i - 1] !== right[j - 1]));
    row = next;
  }
  return row[right.length];
}
function subsequence(query: string, value: string): boolean {
  let index = 0;
  for (const character of value) if (character === query[index]) index++;
  return index === query.length;
}
export function searchSettings(query: string, entries: readonly SettingsSearchEntry[] = SETTINGS_SEARCH_ENTRIES): SettingsSearchEntry[] {
  const needle = normalize(query).slice(0, 120);
  if (!needle) return [];
  const tokens = needle.split(' ');
  return entries.map((entry, index) => {
    const label = normalize(entry.label);
    const fields = [label, ...((entry.keywords ?? []).map(normalize)), normalize(entry.description), normalize(SETTINGS_CATEGORY_LABELS[entry.category as SettingsCategoryId] ?? entry.category)];
    const words = fields.join(' ').split(' ');
    let score = label === needle ? 0 : label.includes(needle) ? 1 : fields.some(field => field.includes(needle)) ? 2 : 3;
    if (score === 3) {
      let fuzzy = false;
      for (const token of tokens) {
        if (words.some(word => word.startsWith(token))) continue;
        const initials = label.split(' ').map(word => word[0]).join('');
        if (token.length >= 2 && (initials.startsWith(token) || (token.length >= 3 && subsequence(token, label.replaceAll(' ', ''))))) { fuzzy = true; continue; }
        if (token.length >= 3 && words.some(word => Math.abs(word.length - token.length) <= 2 && distance(token, word) <= (token.length >= 6 ? 2 : 1))) { fuzzy = true; continue; }
        return { entry, index, score: Infinity };
      }
      score = fuzzy ? 5 : 3;
    }
    return { entry, index, score };
  }).filter(result => Number.isFinite(result.score)).sort((a, b) => a.score - b.score || a.index - b.index).map(result => result.entry);
}
