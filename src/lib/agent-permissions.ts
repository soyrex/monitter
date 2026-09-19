/**
 * Agent Setup permissions and provider model.
 *
 * This module is the single source of truth for the user-facing, normalised
 * representation of an agent's provider and permissions. The stored
 * `Provider` and `Sandbox` types in `$lib/types` are part of the backend
 * contract and must NOT be renamed. They are mapped to/from a UI vocabulary
 * here so the rest of the app can speak in friendly names.
 *
 * Design contract (locked):
 *   - No "ACP" / "native" / "YOLO" string leaks into user-facing copy.
 *   - The user picks one of five providers: Claude Code, Codex, MiniMax,
 *     Gemini, OpenCode. The picker is data-driven and extensible.
 *   - The normalised permission levels are exactly three: read-only,
 *     auto-approve-edits, full-access.
 *   - MiniMax and Gemini are stored as `provider: 'acp'` with the
 *     matching executable configured, because the stored `Provider` enum
 *     does not yet have first-class values for them.
 */

import type { AcpLaunch, Agent, Provider, Sandbox } from '$lib/types';

/** UI-level permission vocabulary. Maps to/from the stored `Sandbox`. */
export type NormalizedPermission = 'read-only' | 'auto-approve-edits' | 'full-access';

export const NORMALIZED_PERMISSIONS: readonly NormalizedPermission[] = [
  'read-only',
  'auto-approve-edits',
  'full-access',
] as const;

/**
 * The logical provider the user sees in the picker and the directory. This
 * is the key the capability table is keyed on. It is intentionally separate
 * from the stored `Provider` enum because MiniMax and Gemini are stored as
 * `provider: 'acp'` and disambiguated by the configured executable name.
 */
export type ProviderKey = 'claude-code' | 'codex' | 'opencode' | 'minimax' | 'gemini';

export interface UiProviderOption {
  /** Logical UI key. Drives the capability table and friendly name lookup. */
  key: ProviderKey;
  /** Friendly display name (NEVER "ACP", "native", or harness jargon). */
  friendlyName: string;
  /** Attribution line shown under the friendly name. */
  subtitle: string;
  /**
   * Stored provider value to write when this tile is selected. For first-
   * class keys (`claude-code`, `codex`, `opencode`) this equals the key's
   * underlying `Provider` enum value. For MiniMax and Gemini we write
   * `acp` together with the executable hint below.
   */
  storedProvider: Provider;
  /**
   * Executable hint written to `agent.acp.command` when this tile is
   * selected. `null` for first-class providers that don't need a launch.
   */
  acpCommand: string | null;
  /** Search keywords used by the extensible picker search. */
  searchTerms: readonly string[];
}

/**
 * The five initial providers exposed in the picker. Additional providers
 * are added by appending entries here; the rest of the module reads from
 * this list rather than hard-coding tile shapes.
 */
export const UI_PROVIDER_CATALOG: readonly UiProviderOption[] = [
  {
    key: 'claude-code',
    friendlyName: 'Claude Code',
    subtitle: 'by Anthropic',
    storedProvider: 'claude',
    acpCommand: null,
    searchTerms: ['anthropic', 'claude'],
  },
  {
    key: 'codex',
    friendlyName: 'Codex',
    subtitle: 'by OpenAI',
    storedProvider: 'codex',
    acpCommand: null,
    searchTerms: ['openai', 'gpt', 'codex'],
  },
  {
    key: 'opencode',
    friendlyName: 'OpenCode',
    subtitle: 'open source',
    storedProvider: 'opencode',
    acpCommand: null,
    searchTerms: ['opencode', 'oss'],
  },
  {
    key: 'minimax',
    friendlyName: 'MiniMax',
    subtitle: 'by MiniMax',
    storedProvider: 'acp',
    acpCommand: 'mcode',
    searchTerms: ['minimax', 'mcode', 'MiniMax'],
  },
  {
    key: 'gemini',
    friendlyName: 'Gemini',
    subtitle: 'by Google',
    storedProvider: 'acp',
    acpCommand: 'gemini',
    searchTerms: ['gemini', 'google'],
  },
];

export function getProviderOption(key: ProviderKey): UiProviderOption {
  const match = UI_PROVIDER_CATALOG.find(option => option.key === key);
  if (!match) throw new Error(`Unknown provider key: ${key}`);
  return match;
}

/** Map an agent's stored data to the logical UI provider key. */
export function deriveProviderKey(agent: Pick<Agent, 'provider' | 'acp'>): ProviderKey {
  if (agent.provider === 'claude') return 'claude-code';
  if (agent.provider === 'codex') return 'codex';
  if (agent.provider === 'opencode') return 'opencode';
  if (agent.provider === 'hermes') return 'opencode';
  if (agent.provider === 'acp') {
    return detectProviderFromLaunch(agent.acp ?? null);
  }
  return 'opencode';
}

/**
 * Best-effort provider detection from an ACP launch's executable. Used
 * for the silent migration of legacy agents that stored every non-built-in
 * provider as `provider: 'acp'`. The fallback is `opencode` so the UI
 * never shows an unknown state.
 */
export function detectProviderFromLaunch(launch: AcpLaunch | null | undefined): ProviderKey {
  const command = (launch?.command ?? '').toLowerCase();
  if (!command) return 'opencode';
  if (/(?:^|[\W_])(?:mcode|minimax)(?:$|[\W_])/.test(command)) return 'minimax';
  if (/(?:^|[\W_])gemini(?:$|[\W_])/.test(command)) return 'gemini';
  if (/(?:^|[\W_])claude(?:$|[\W_])/.test(command)) return 'claude-code';
  if (/(?:^|[\W_])codex(?:$|[\W_])/.test(command)) return 'codex';
  if (/(?:^|[\W_])opencode(?:$|[\W_])/.test(command)) return 'opencode';
  return 'opencode';
}

export interface PermissionCapability {
  /** Whether "Read only" is supported for this provider. */
  readOnly: boolean;
  /** Whether "Auto-approve edits" is supported for this provider. */
  autoApproveEdits: boolean;
  /** Whether "Full access" is supported for this provider. */
  fullAccess: boolean;
  /** Default level applied when the user has not picked one yet. */
  defaultLevel: NormalizedPermission;
  /**
   * Single-sentence hint rendered below the permission radio cards.
   * Always written in plain language; no harness jargon.
   */
  hint: string;
  /**
   * Optional reason a disabled level is disabled. Shown next to the
   * disabled card so users see why they can't pick it.
   */
  disabledReason?: Partial<Record<NormalizedPermission, string>>;
}

/**
 * Per-provider permission capability table. The locked values from the
 * product decisions are encoded here so the UI never has to encode them
 * again.
 */
export const PERMISSION_CAPABILITIES: Record<ProviderKey, PermissionCapability> = {
  'claude-code': {
    readOnly: true,
    autoApproveEdits: true,
    fullAccess: true,
    defaultLevel: 'auto-approve-edits',
    hint: 'Claude Code supports all three levels.',
  },
  codex: {
    readOnly: true,
    autoApproveEdits: true,
    fullAccess: true,
    defaultLevel: 'read-only',
    hint: 'Codex supports all three levels.',
  },
  opencode: {
    readOnly: true,
    autoApproveEdits: true,
    fullAccess: true,
    defaultLevel: 'auto-approve-edits',
    hint: 'OpenCode supports all three levels.',
  },
  minimax: {
    readOnly: true,
    autoApproveEdits: true,
    fullAccess: true,
    defaultLevel: 'auto-approve-edits',
    hint: 'MiniMax supports all three levels.',
  },
  gemini: {
    readOnly: false,
    autoApproveEdits: true,
    fullAccess: true,
    defaultLevel: 'auto-approve-edits',
    hint: "Gemini doesn't support Read only — its lowest setting still allows edits.",
  },
};

export function capabilityFor(agent: Pick<Agent, 'provider' | 'acp'>): PermissionCapability {
  return PERMISSION_CAPABILITIES[deriveProviderKey(agent)];
}

export function isLevelSupported(agent: Pick<Agent, 'provider' | 'acp'>, level: NormalizedPermission): boolean {
  const caps = capabilityFor(agent);
  switch (level) {
    case 'read-only': return caps.readOnly;
    case 'auto-approve-edits': return caps.autoApproveEdits;
    case 'full-access': return caps.fullAccess;
  }
}

/**
 * Convert a stored `Sandbox` value to the normalised permission that the
 * picker should display for the given provider. Used for the silent
 * permission migration: existing values stay valid, but the UI presents
 * them through the new vocabulary. If a level isn't supported for the
 * provider, falls back to the closest supported level above it, then to
 * the default.
 */
export function normalizeStoredSandbox(agent: Pick<Agent, 'provider' | 'acp' | 'sandbox'>): NormalizedPermission {
  const caps = capabilityFor(agent);
  const fromStored = (() => {
    switch (agent.sandbox) {
      case 'read-only': return 'read-only' as NormalizedPermission;
      case 'workspace-write': return 'auto-approve-edits' as NormalizedPermission;
      case 'yolo': return 'full-access' as NormalizedPermission;
      case 'harness-configured':
        // The harness-configured bucket is provider-implicit. Codex means
        // read-only, everyone else means auto-approve-edits.
        return agent.provider === 'codex'
          ? ('read-only' as NormalizedPermission)
          : ('auto-approve-edits' as NormalizedPermission);
    }
  })();
  if (isLevelSupported(agent, fromStored)) return fromStored;
  if (caps.autoApproveEdits) return 'auto-approve-edits';
  return caps.fullAccess ? 'full-access' : caps.defaultLevel;
}

/**
 * Convert the normalised picker value back to a stored `Sandbox` value.
 * This is the only place the harness-specific vocabulary is allowed to
 * exist; it must never appear in user-facing strings.
 *
 * Locked mapping (per product decisions):
 *   read-only         -> 'read-only'
 *   auto-approve-edits-> 'workspace-write' for Codex, 'harness-configured' elsewhere
 *   full-access       -> 'yolo'
 */
export function denormalizePermission(provider: Provider, level: NormalizedPermission): Sandbox {
  switch (level) {
    case 'read-only':
      return 'read-only';
    case 'auto-approve-edits':
      return provider === 'codex' ? 'workspace-write' : 'harness-configured';
    case 'full-access':
      return 'yolo';
  }
}

/**
 * If the picked level is not supported for the current provider, fall
 * back to the closest supported level. Used when the user changes the
 * provider mid-edit and the previous level is no longer available.
 */
export function clampLevelToCapability(
  agent: Pick<Agent, 'provider' | 'acp'>,
  preferred: NormalizedPermission,
): NormalizedPermission {
  if (isLevelSupported(agent, preferred)) return preferred;
  const caps = capabilityFor(agent);
  if (preferred === 'full-access' && caps.autoApproveEdits) return 'auto-approve-edits';
  if (preferred === 'auto-approve-edits' && caps.readOnly) return 'read-only';
  return caps.defaultLevel;
}

/**
 * Apply a provider picker selection to an agent draft in-place. Updates
 * `provider` and (for ACP-backed presets) `acp`, and clamps the current
 * normalised permission to one the new provider supports.
 */
export function applyProviderChoice(agent: Agent, key: ProviderKey): void {
  const option = getProviderOption(key);
  agent.provider = option.storedProvider;
  if (option.acpCommand) {
    const existing = agent.acp ?? { command: '', args: [] };
    agent.acp = { command: option.acpCommand, args: existing.args ?? [] };
  } else if (agent.provider !== 'acp') {
    // Clear any stale ACP launch when switching to a first-class provider.
    agent.acp = null;
  }
  // Reset model so the user re-picks one for the new provider.
  agent.model = '';
}

/** Friendly name for a stored agent. Used in the directory and pickers. */
export function friendlyProviderName(agent: Pick<Agent, 'provider' | 'acp'>): string {
  return getProviderOption(deriveProviderKey(agent)).friendlyName;
}

/** Human-readable permission label. Used in directory rows and pickers. */
export function friendlyPermissionLabel(level: NormalizedPermission): string {
  switch (level) {
    case 'read-only': return 'Read only';
    case 'auto-approve-edits': return 'Auto-approve edits';
    case 'full-access': return 'Full access';
  }
}

/** One-line plain-language description for the picker radio cards. */
export function permissionDescription(level: NormalizedPermission): string {
  switch (level) {
    case 'read-only':
      return 'Can read files. Can\u2019t make changes.';
    case 'auto-approve-edits':
      return 'Edits workspace files without asking. Won\u2019t run arbitrary commands.';
    case 'full-access':
      return 'Can run any command, edit anything, no prompts.';
  }
}
