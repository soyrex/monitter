import { COMMAND_CONTRACT_PROTOCOL_VERSION, LEGACY_COMPATIBLE_COMMANDS } from './generated-command-contract';
import type { CommandCapabilities } from './types';

const legacyCommands = new Set<string>(LEGACY_COMPATIBLE_COMMANDS);

function missingHandshake(reason: unknown): boolean {
  const message = String(reason).toLowerCase();
  return message.includes('get_command_capabilities') && (
    message.includes('not found') || message.includes('unknown command') ||
    message.includes('unknown invoke command') || message.includes('is not available')
  );
}

/** One guard per transport. It coalesces simultaneous probes but never caches
 * a protocol rejection or silently retries an operation after dispatch. */
export function createCommandGuard(probe: () => Promise<unknown>) {
  let pending: Promise<CommandCapabilities | null> | undefined;

  function capabilities(): Promise<CommandCapabilities | null> {
    if (pending) return pending;
    pending = probe().then(value => {
      if (!value || typeof value !== 'object' || !('protocolVersion' in value) || !('commands' in value)) {
        throw new Error('Monitter returned an invalid command compatibility response.');
      }
      if (value.protocolVersion !== COMMAND_CONTRACT_PROTOCOL_VERSION) {
        throw new Error(`Unsupported Monitter command protocol ${String(value.protocolVersion)}; this client supports ${COMMAND_CONTRACT_PROTOCOL_VERSION}.`);
      }
      if (!Array.isArray(value.commands) || !value.commands.every(command => typeof command === 'string')) {
        throw new Error('Monitter returned an invalid command capability list.');
      }
      return { protocolVersion: value.protocolVersion, commands: value.commands };
    }).catch(reason => {
      if (missingHandshake(reason)) return null;
      throw reason;
    }).finally(() => { pending = undefined; });
    return pending;
  }

  return async (command: string): Promise<void> => {
    // The developer bridge's sole native escape hatch must work even when
    // that origin cannot invoke the compatibility probe. Both bootstrap
    // commands have stable empty argument lists and keep their own Tauri ACL.
    if (command === 'get_command_capabilities' || command === 'use_packaged_ui') return;
    const result = await capabilities();
    if (result ? !result.commands.includes(command) : !legacyCommands.has(command)) {
      throw new Error(`This Monitter backend does not support the command '${command}'. Refresh or update the desktop app.`);
    }
  };
}
