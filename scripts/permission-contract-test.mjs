import { readFileSync } from 'node:fs';

const source = readFileSync('src-tauri/src/lib.rs', 'utf8');
const permissions = readFileSync('src-tauri/permissions/default.toml', 'utf8');
const handler = source.slice(source.lastIndexOf('tauri::generate_handler!['));
const registered = [...handler.slice(0, handler.indexOf('])')).matchAll(/^\s{12}([a-z][a-z0-9_]*)[,]?$/gm)].map((match) => match[1]);
const granted = [...permissions.matchAll(/"([a-z][a-z0-9_]*)"/g)].map((match) => match[1]);
const missing = registered.filter((command) => !granted.includes(command));
const stale = granted.filter((command) => !registered.includes(command));

if (missing.length || stale.length) {
  throw new Error(`Tauri command ACL drift. Missing: ${missing.join(', ') || 'none'}; stale: ${stale.join(', ') || 'none'}.`);
}
const lanStart = source.indexOf('pub(crate) fn lan_invoke');
const lanEnd = source.indexOf('pub(crate) fn create_approval_request', lanStart);
if (lanStart < 0 || lanEnd < 0) {
  throw new Error('Could not locate the explicit LAN command allow-list.');
}
const lanDispatcher = source.slice(lanStart, lanEnd);
if (lanDispatcher.includes('"plan_jev_command"')) {
  throw new Error('plan_jev_command must remain native-owner only and absent from LAN dispatch.');
}
console.log(`Tauri command ACL covers all ${registered.length} registered commands.`);
