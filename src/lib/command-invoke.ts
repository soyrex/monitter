import { invoke as nativeInvoke } from '@tauri-apps/api/core';
import type { CommandArgs, CommandName, CommandResults } from './generated-command-contract';
import { createCommandGuard } from './command-protocol';

const requireCompatibleCommand = createCommandGuard(() => nativeInvoke('get_command_capabilities'));

/** Typed Tauri call surface generated from the native Rust command signatures. */
export async function invokeCommand<C extends CommandName>(
  command: C,
  ...args: keyof CommandArgs[C] extends never ? [args?: CommandArgs[C]] : [args: CommandArgs[C]]
): Promise<CommandResults[C]> {
  await requireCompatibleCommand(command);
  return nativeInvoke<CommandResults[C]>(command, (args[0] ?? {}) as Record<string, unknown>);
}
