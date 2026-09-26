import { invoke as nativeInvoke } from '@tauri-apps/api/core';
import type { CommandArgs, CommandName, CommandResults } from './generated-command-contract';

/** Typed Tauri call surface generated from the native Rust command signatures. */
export function invokeCommand<C extends CommandName>(
  command: C,
  ...args: keyof CommandArgs[C] extends never ? [args?: CommandArgs[C]] : [args: CommandArgs[C]]
): Promise<CommandResults[C]> {
  return nativeInvoke<CommandResults[C]>(command, (args[0] ?? {}) as Record<string, unknown>);
}
