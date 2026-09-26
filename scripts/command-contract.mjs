import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';

const root = process.cwd();
const manifestPath = join(root, 'command-contract.json');
const generatedPath = join(root, 'src/lib/generated-command-contract.ts');
const rustRoot = join(root, 'src-tauri/src');
const TYPE_OVERRIDES = {
  'UiSnapshot': 'UiSnapshotResponse', 'lan::Info': 'LanServerInfo', 'ModelCatalogTarget': 'ModelTarget',
  'attachments::ReadAttachmentFile': 'AttachmentFileData', 'voice::VoiceTranscript': 'VoiceTranscription',
  'deletion::Preview': 'TaskDeletionPreview', 'deletion::DeletionPreview': 'TaskDeletionPreview',
  'lan_sync::Accepted': 'SendAccepted',
  'git::GitStatus': 'TaskGitStatus', 'git::GitDiff': 'TaskGitDiff',
  'BrowserExtensionState': 'BrowserExtensionLoadResult',
  'ScheduleInput': 'Schedule',
  'acp_probe::ProbeResult': 'AcpProbeResult',
};
const ARG_TYPE_OVERRIDES = {
  'get_usage_overview.policy': 'UsageRefreshPolicy',
  'resolve_approval.decision': 'ApprovalDecision',
  'get_task_git_diff.scope': 'GitDiffScope',
  'set_task_sandbox.sandbox': 'Sandbox',
};
const STRUCTURAL_TYPES = [
  'Host', 'Agent', 'Task', 'Message', 'Project', 'Channel', 'Settings', 'Snapshot',
  'CreateTaskInput', 'HandoffTaskInput', 'ForkTaskInput',
];
const toCamel = value => value.replace(/_([a-z])/g, (_, letter) => letter.toUpperCase());
const fail = message => { throw new Error(`Command contract: ${message}`); };

function rustFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? rustFiles(path) : entry.name.endsWith('.rs') ? [path] : [];
  });
}

function matching(source, open, left = '(', right = ')') {
  let depth = 0;
  for (let index = open; index < source.length; index += 1) {
    if (source[index] === left) depth += 1;
    else if (source[index] === right && --depth === 0) return index;
  }
  return -1;
}

function splitTopLevel(value, delimiter = ',') {
  const parts = [];
  let angle = 0, paren = 0, brace = 0, bracket = 0, start = 0;
  for (let index = 0; index < value.length; index += 1) {
    const ch = value[index];
    if (ch === '<') angle += 1;
    else if (ch === '>') angle -= 1;
    else if (ch === '(') paren += 1;
    else if (ch === ')') paren -= 1;
    else if (ch === '{') brace += 1;
    else if (ch === '}') brace -= 1;
    else if (ch === '[') bracket += 1;
    else if (ch === ']') bracket -= 1;
    else if (ch === delimiter && !angle && !paren && !brace && !bracket) {
      parts.push(value.slice(start, index).trim()); start = index + 1;
    }
  }
  if (value.slice(start).trim()) parts.push(value.slice(start).trim());
  return parts;
}

function rustCommands() {
  const found = new Map();
  for (const file of rustFiles(rustRoot)) {
    const source = readFileSync(file, 'utf8');
    const marker = /#\[tauri::command(?:\([^\]]*\))?\]\s*(?:(?:pub(?:\([^)]*\))?)\s*)?(?:async\s+)?fn\s+([a-z][a-z0-9_]*)\s*\(/g;
    for (const match of source.matchAll(marker)) {
      const name = match[1];
      const open = match.index + match[0].lastIndexOf('(');
      const close = matching(source, open);
      if (close < 0) fail(`cannot parse Rust arguments for ${name} in ${relative(root, file)}`);
      const args = [];
      for (const part of splitTopLevel(source.slice(open + 1, close))) {
        const colon = part.indexOf(':');
        if (colon < 0) fail(`cannot parse Rust parameter '${part}' for ${name}`);
        const rustName = part.slice(0, colon).trim().replace(/^mut\s+/, '');
        const rustType = part.slice(colon + 1).trim().replace(/\s+/g, ' ');
        if (/^(?:state|app|webview|window)\b/.test(rustName) || /(?:^|::)State\s*<|(?:^|::)AppHandle\b|(?:^|::)Webview(?:Window)?\s*</.test(rustType)) continue;
        args.push({ name: toCamel(rustName), rustName, rustType });
      }
      const tail = source.slice(close + 1);
      const resultMatch = /^\s*->\s*([^\{]+)\{/.exec(tail);
      const rawResult = resultMatch?.[1]?.trim().replace(/\s+/g, ' ') ?? '()';
      const result = /^Result\s*<([\s\S]*)>$/.exec(rawResult);
      let resultType = rawResult;
      if (result) {
        const resultParts = splitTopLevel(result[1]);
        if (resultParts.length !== 2 || resultParts[1] !== 'String') fail(`expected Result<T, String> for ${name}, got ${rawResult}`);
        resultType = resultParts[0];
      }
      typeName(resultType);
      if (found.has(name)) fail(`duplicate Tauri command definition ${name}`);
      found.set(name, { args, resultRustType: resultType });
    }
  }
  return found;
}

function inventories() {
  const lib = readFileSync(join(rustRoot, 'lib.rs'), 'utf8');
  const start = lib.lastIndexOf('tauri::generate_handler![');
  if (start < 0) fail('cannot find generate_handler inventory');
  const end = lib.indexOf('])', start);
  if (end < 0) fail('cannot find end of generate_handler inventory');
  const registered = [...lib.slice(start, end).matchAll(/^\s{12}((?:[a-z][a-z0-9_]*::)*[a-z][a-z0-9_]*)[,]?$/gm)]
    .map(match => match[1].split('::').at(-1));
  const lanStart = lib.indexOf('pub(crate) fn lan_invoke');
  const lanEnd = lib.indexOf('pub(crate) fn create_approval_request', lanStart);
  if (lanStart < 0 || lanEnd < 0) fail('cannot locate explicit LAN dispatcher');
  const lanSource = lib.slice(lanStart, lanEnd);
  const lan = [...lanSource.matchAll(/^\s*"([a-z][a-z0-9_]*)"\s*=>/gm)].map(match => match[1]);
  const permissions = readFileSync(join(root, 'src-tauri/permissions/default.toml'), 'utf8');
  const localAcl = /identifier\s*=\s*"allow-local-monitter"[\s\S]*?commands\.allow\s*=\s*\[([\s\S]*?)\]/.exec(permissions)?.[1];
  if (!localAcl) fail('cannot locate allow-local-monitter ACL');
  const granted = [...localAcl.matchAll(/"([a-z][a-z0-9_]*)"/g)].map(match => match[1]);
  return { registered, lan, granted };
}

function typeName(rustType) {
  const value = rustType.trim().replace(/^&'?\w+\s+/, '');
  if (value === '()') return 'void';
  if (value === 'String' || value === '&str' || value === 'str') return 'string';
  if (value === 'bool') return 'boolean';
  if (/^(?:u|i)(?:8|16|32|64|128|size)$/.test(value)) return 'number';
  if (value === 'serde_json::Value' || value === 'Value') return 'JsonValue';
  const generic = /^([\w:]+)\s*<([\s\S]*)>$/.exec(value);
  if (generic) {
    const args = splitTopLevel(generic[2]);
    const inner = args[0];
    if (generic[1].endsWith('Option') && args.length === 1) return `${typeName(inner)} | null`;
    if (generic[1].endsWith('Vec') && args.length === 1) return `${typeName(inner)}[]`;
    if ((generic[1].endsWith('HashMap') || generic[1].endsWith('BTreeMap')) && args.length === 2) return `Record<${typeName(args[0])}, ${typeName(args[1])}>`;
    fail(`unsupported Rust type ${value}`);
  }
  const name = TYPE_OVERRIDES[value] ?? TYPE_OVERRIDES[value.split('::').at(-1)] ?? value.split('::').at(-1);
  if (/^[A-Z][A-Za-z0-9_]*$/.test(name ?? '')) return name;
  fail(`unsupported Rust type ${value}`);
}

function argType(arg, command) {
  const optional = /^Option\s*</.test(arg.rustType);
  return { ...arg, tsType: ARG_TYPE_OVERRIDES[`${command}.${arg.name}`] ?? typeName(arg.rustType), optional };
}

function buildManifest(oldManifest, commands, inventory) {
  const old = oldManifest?.commands ?? {};
  const result = {};
  for (const name of inventory.registered) {
    const signature = commands.get(name);
    if (!signature) fail(`registered command ${name} has no parsed #[tauri::command] definition`);
    const prior = old[name];
    result[name] = {
      arguments: signature.args.map(arg => argType(arg, name)),
      resultRustType: signature.resultRustType,
      resultTsType: typeName(signature.resultRustType),
      surfaces: prior?.surfaces ?? (oldManifest ? ['native'] : inventory.lan.includes(name) ? ['native', 'lan'] : ['native']),
    };
  }
  const defaults = {
    controller: {
      actions: ['getSnapshot', 'sendMessage', 'beginAttachmentUpload', 'appendAttachmentUpload', 'finishAttachmentUpload', 'cancelTask', 'resumeTask', 'listTerminals', 'readTerminal'],
      mappings: {
        getSnapshot: { nativeCommands: ['get_snapshot', 'get_ui_snapshot'], semantics: 'read snapshot then bound event details for controller response' },
        sendMessage: { nativeCommands: ['send_message', 'send_message_fast'], semantics: 'send once; legacy clients receive a snapshot, negotiated clients may receive acceptance receipt' },
        beginAttachmentUpload: { nativeCommands: [], semantics: 'allocate bounded per-peer in-memory upload; no native call' },
        appendAttachmentUpload: { nativeCommands: [], semantics: 'append bounded chunk to per-peer in-memory upload; no native call' },
        finishAttachmentUpload: { nativeCommands: ['store_attachment'], semantics: 'assemble upload then persist through owner bridge' },
        cancelTask: { nativeCommands: ['cancel_task'], semantics: 'owner bridge cancellation followed by bounded snapshot' },
        resumeTask: { nativeCommands: ['resume_task'], semantics: 'owner bridge resume followed by bounded snapshot' },
        listTerminals: { nativeCommands: ['list_terminals'], semantics: 'owner bridge terminal list' },
        readTerminal: { nativeCommands: ['read_terminal'], semantics: 'owner bridge terminal read' },
      },
    },
    visitor: {
      allowedActions: ['getSnapshot', 'sendMessage', 'storeAttachment'],
      deniedActions: ['cancelTask', 'resumeTask', 'listTerminals', 'readTerminal'],
      mappings: {
        getSnapshot: { nativeCommands: ['get_snapshot', 'get_ui_snapshot'], semantics: 'read then sharedSnapshot privacy projection' },
        sendMessage: { nativeCommands: ['get_snapshot', 'get_ui_snapshot', 'send_message', 'send_message_fast'], semantics: 'check shared scope and compose tagged visitor message before owner bridge send' },
        storeAttachment: { nativeCommands: ['get_snapshot', 'get_ui_snapshot', 'store_attachment'], semantics: 'check shared task scope before owner bridge attachment storage' },
      },
    },
  };
  const protocols = {
    controller: { ...defaults.controller, ...(oldManifest?.protocols?.controller ?? {}) },
    visitor: { ...defaults.visitor, ...(oldManifest?.protocols?.visitor ?? {}) },
  };
  if (!protocols.controller?.actions?.length) protocols.controller = defaults.controller;
  if (!protocols.visitor?.allowedActions?.length) protocols.visitor = defaults.visitor;
  const nominalTypes = new Set();
  for (const entry of Object.values(result)) {
    for (const arg of entry.arguments) for (const type of arg.tsType.match(/[A-Z][A-Za-z0-9_]*/g) ?? []) if (type !== 'Record') nominalTypes.add(type);
    for (const type of entry.resultTsType.match(/[A-Z][A-Za-z0-9_]*/g) ?? []) if (type !== 'Record') nominalTypes.add(type);
  }
  const unsupportedMemberTypes = [...nominalTypes].filter(type => type !== 'JsonValue' && !STRUCTURAL_TYPES.includes(type)).sort();
  return {
    protocolVersion: oldManifest?.protocolVersion ?? 1,
    commands: result,
    protocols,
    memberParity: { checked: STRUCTURAL_TYPES, unsupported: unsupportedMemberTypes },
  };
}

function generatedType(rustType) { return typeName(rustType); }

function generateTs(manifest) {
  const custom = new Set();
  for (const entry of Object.values(manifest.commands)) {
    for (const arg of entry.arguments) for (const name of arg.tsType.match(/[A-Z][A-Za-z0-9_]*/g) ?? []) if (!['Record'].includes(name)) custom.add(name);
    for (const name of entry.resultTsType.match(/[A-Z][A-Za-z0-9_]*/g) ?? []) if (!['Record'].includes(name)) custom.add(name);
  }
  const types = readFileSync(join(root, 'src/lib/types.ts'), 'utf8');
  for (const name of custom) if (name !== 'JsonValue' && !new RegExp(`\\b(?:interface|type|enum)\\s+${name}\\b`).test(types)) fail(`generated Rust type ${name} has no TypeScript definition in src/lib/types.ts`);
  const imports = [...custom].sort().join(', ');
  let output = `// Generated by scripts/command-contract.mjs from Rust Tauri command signatures.\nimport type { ${imports} } from './types';\n\nexport const COMMAND_CONTRACT_PROTOCOL_VERSION = ${manifest.protocolVersion} as const;\n\n`;
  output += 'export interface CommandArgs {\n';
  for (const [name, entry] of Object.entries(manifest.commands)) {
    const fields = entry.arguments.map(arg => `  ${arg.name}${arg.optional ? '?' : ''}: ${arg.tsType};`).join('\n');
    output += `  ${JSON.stringify(name)}: {${fields ? `\n${fields}\n` : ''}  };\n`;
  }
  output += '}\n\nexport interface CommandResults {\n';
  for (const [name, entry] of Object.entries(manifest.commands)) output += `  ${JSON.stringify(name)}: ${entry.resultTsType};\n`;
  output += '}\n\nexport type CommandName = keyof CommandArgs;\n';
  return output;
}

function compareSource(manifest, inventory, sourceCommands) {
  const listed = Object.keys(manifest.commands).sort();
  const registered = [...inventory.registered].sort();
  const granted = [...inventory.granted].sort();
  const lan = [...inventory.lan].sort();
  const expectedLan = listed.filter(name => manifest.commands[name].surfaces.includes('lan')).sort();
  for (const [label, left, right] of [['registered commands vs manifest', registered, listed], ['native ACL vs manifest', granted, listed], ['LAN dispatcher vs manifest surface', lan, expectedLan]]) {
    if (JSON.stringify(left) !== JSON.stringify(right)) fail(`${label} differ. expected manifest=${right.join(', ')} actual=${left.join(', ')}`);
  }
  for (const [name, entry] of Object.entries(manifest.commands)) {
    if (!entry.surfaces.includes('native') || entry.surfaces.some(surface => !['native', 'lan'].includes(surface))) fail(`${name} has invalid surface declaration`);
    const actual = sourceCommands.get(name);
    if (!actual) fail(`${name} has no Rust command definition`);
    const normalize = args => args.map(({ name: argName, rustType }) => `${argName}:${rustType}`).join('|');
    const expectedArgs = normalize(entry.arguments);
    const actualArgs = normalize(actual.args.map(arg => argType(arg, name)));
    if (expectedArgs !== actualArgs || entry.resultRustType !== actual.resultRustType || entry.resultTsType !== typeName(actual.resultRustType)) fail(`${name} Rust signature differs from manifest (args ${expectedArgs} vs ${actualArgs}; result ${entry.resultRustType}/${entry.resultTsType} vs ${actual.resultRustType}/${typeName(actual.resultRustType)})`);
  }
  const rustModel = readFileSync(join(rustRoot, 'model.rs'), 'utf8');
  const tsTypes = readFileSync(join(root, 'src/lib/types.ts'), 'utf8');
  for (const name of manifest.memberParity?.checked ?? []) {
    const rustMarker = new RegExp(`pub struct ${name}\\s*\\{`).exec(rustModel);
    const tsMarker = new RegExp(`export interface ${name}\\s*\\{`).exec(tsTypes);
    if (!rustMarker || !tsMarker) fail(`member parity cannot locate Rust/TypeScript ${name}`);
    const rustOpen = rustModel.indexOf('{', rustMarker.index);
    const tsOpen = tsTypes.indexOf('{', tsMarker.index);
    const rustEnd = matching(rustModel, rustOpen, '{', '}');
    const tsEnd = matching(tsTypes, tsOpen, '{', '}');
    if (rustEnd < 0 || tsEnd < 0) fail(`member parity cannot parse ${name}`);
    const rustMembers = rustModel.slice(rustOpen + 1, rustEnd).replace(/^\s*#\[[^\]]*\]\s*/gm, '').replace(/^\s*\/\/\/.*$/gm, '');
    const rustFields = splitTopLevel(rustMembers).flatMap(part => {
      const field = /^\s*pub\s+([a-z][a-z0-9_]*)\s*:/s.exec(part);
      return field ? [toCamel(field[1])] : [];
    }).sort();
    const tsMembers = tsTypes.slice(tsOpen + 1, tsEnd).replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
    const tsFields = splitTopLevel(tsMembers, ';').flatMap(part => {
      const field = /^\s*([a-z][A-Za-z0-9_]*)\??\s*:/s.exec(part);
      return field ? [field[1]] : [];
    }).sort();
    if (JSON.stringify(rustFields) !== JSON.stringify(tsFields)) fail(`${name} Rust/TypeScript members differ (Rust: ${rustFields.join(', ')}; TypeScript: ${tsFields.join(', ')})`);
  }
  const actualUnsupported = new Set();
  for (const entry of Object.values(manifest.commands)) {
    for (const field of [...entry.arguments.map(arg => arg.tsType), entry.resultTsType]) {
      for (const type of field.match(/[A-Z][A-Za-z0-9_]*/g) ?? []) if (!['Record', 'JsonValue', ...STRUCTURAL_TYPES].includes(type)) actualUnsupported.add(type);
    }
  }
  if (JSON.stringify([...actualUnsupported].sort()) !== JSON.stringify(manifest.memberParity?.unsupported ?? [])) fail('memberParity.unsupported must explicitly list every command-bound nominal type without Rust/TypeScript member-field parity checks');
  const controllerProtocol = readFileSync(join(root, 'src/lib/controller/protocol.ts'), 'utf8');
  const controllerDispatcher = readFileSync(join(root, 'src/lib/controller/dispatcher.ts'), 'utf8');
  const protocolActions = [...controllerProtocol.matchAll(/action:\s*'([a-z][A-Za-z0-9]+)'/g)].map(match => match[1]);
  const dispatchActions = [...controllerDispatcher.matchAll(/case\s+'([a-z][A-Za-z0-9]+)'\s*:/g)].map(match => match[1]);
  const declaredActions = manifest.protocols?.controller?.actions ?? [];
  const normalized = values => [...new Set(values)].sort();
  if (JSON.stringify(normalized(protocolActions)) !== JSON.stringify(normalized(declaredActions)) || JSON.stringify(normalized(dispatchActions)) !== JSON.stringify(normalized(declaredActions))) fail('controller protocol actions differ from the manifest');
  const controllerMappings = manifest.protocols?.controller?.mappings ?? {};
  if (JSON.stringify(normalized(Object.keys(controllerMappings))) !== JSON.stringify(normalized(declaredActions))) fail('controller mappings must explicitly describe every protocol action');
  for (const [action, mapping] of Object.entries(controllerMappings)) for (const command of mapping.nativeCommands ?? []) {
    if (!manifest.commands[command]?.surfaces?.includes('native')) fail(`controller mapping ${action} references command ${command} outside the native surface`);
  }
  const visitor = readFileSync(join(root, 'src/lib/operator-sharing.ts'), 'utf8');
  for (const action of manifest.protocols?.visitor?.allowedActions ?? []) if (!visitor.includes(action)) fail(`visitor manifest action ${action} is absent from operator-sharing.ts`);
  for (const action of manifest.protocols?.visitor?.deniedActions ?? []) if (!new RegExp(`${action}:\\s*denied`).test(visitor)) fail(`visitor must keep ${action} denied`);
  const visitorMappings = manifest.protocols?.visitor?.mappings ?? {};
  if (JSON.stringify(normalized(Object.keys(visitorMappings))) !== JSON.stringify(normalized(manifest.protocols?.visitor?.allowedActions ?? []))) fail('visitor mappings must explicitly describe every allowed action and no denied action');
  for (const [action, mapping] of Object.entries(visitorMappings)) for (const command of mapping.nativeCommands ?? []) {
    if (!manifest.commands[command]?.surfaces?.includes('native')) fail(`visitor mapping ${action} references command ${command} outside the native surface`);
  }
  if ([...Object.entries(manifest.commands)].some(([name, entry]) => entry.surfaces.includes('visitor') || entry.surfaces.includes('controller'))) fail('Tauri commands cannot be directly exposed to visitor/controller transports; declare those protocol mappings separately');
  const bridge = readFileSync(join(root, 'src/lib/bridge.ts'), 'utf8');
  if (!/invoke\('get_command_capabilities'\)[\s\S]*?result\.protocolVersion\s*!==\s*COMMAND_CONTRACT_PROTOCOL_VERSION/.test(bridge)) fail('bridge must negotiate and reject incompatible command protocol versions before using capabilities');
  if (!/async function chooseSendProtocol\(\)[\s\S]*?await getCommandCapabilities\(\)[\s\S]*?send_message_fast/.test(bridge)) fail('send protocol selection must negotiate capabilities before selecting the fast mutation');
  for (const name of ['sendMessageWithProtocol', 'sendChannelMessageWithProtocol']) {
    const start = bridge.indexOf(`async function ${name}(`);
    const end = bridge.indexOf('\n}', start);
    if (start < 0 || end < 0) fail(`cannot verify mutation retry behavior for ${name}`);
    const body = bridge.slice(start, end);
    if (/catch\s*\(/.test(body)) fail(`${name} must not retry a mutation after an ambiguous fast-command failure`);
  }
}

const inventory = inventories();
const sourceCommands = rustCommands();
let oldManifest = null;
try { oldManifest = JSON.parse(readFileSync(manifestPath, 'utf8')); } catch {}
const manifest = buildManifest(oldManifest, sourceCommands, inventory);
const generated = generateTs(manifest);
if (process.argv.includes('--write')) {
  writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
  writeFileSync(generatedPath, generated);
} else {
  compareSource(manifest, inventory, sourceCommands);
  if (readFileSync(generatedPath, 'utf8') !== generated) fail('generated TypeScript is stale; run node scripts/command-contract.mjs --write');
  console.log(`Command contract covers ${Object.keys(manifest.commands).length} native commands (${inventory.lan.length} LAN commands).`);
}
