#!/usr/bin/env node
// A deterministic Pi RPC double. It never runs a model, tool, or user code.
import { createInterface } from 'node:readline';
import { appendFileSync, existsSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const scratch = process.env.MONITTER_PI_SMOKE_DIR;
if (!scratch) throw new Error('Missing smoke scratch folder.');
const sessionId = '11111111-2222-4333-8444-555555555555';
const sessionFile = join(scratch, 'fixture-session.jsonl');
const model = { provider: 'fixture', id: 'offline', name: 'Offline test model', reasoning: false };
if (!existsSync(sessionFile)) writeFileSync(sessionFile, '{}\n');
appendFileSync(join(scratch, 'rpc-log.jsonl'), JSON.stringify({ type: 'spawn', args: process.argv.slice(2) }) + '\n');
const input = createInterface({ input: process.stdin });
input.on('line', line => {
  const request = JSON.parse(line);
  appendFileSync(join(scratch, 'rpc-log.jsonl'), JSON.stringify({ type: request.type }) + '\n');
  let data;
  switch (request.type) {
    case 'get_state': data = { sessionId, sessionFile, model, thinkingLevel: 'off', isStreaming: false }; break;
    case 'get_available_models': data = { models: [model] }; break;
    case 'get_commands': data = { commands: [] }; break;
    case 'get_messages': data = { messages: [
      { role: 'user', content: [{ type: 'text', text: 'Fixture saved question' }] },
      { role: 'assistant', content: [{ type: 'text', text: 'Fixture saved reply' }] },
    ] }; break;
    default: {
      process.stdout.write(JSON.stringify({ type: 'response', id: request.id, command: request.type,
        success: false, error: 'Unexpected RPC command; no prompts or tools are allowed.' }) + '\n');
      return;
    }
  }
  process.stdout.write(JSON.stringify({ type: 'response', id: request.id, command: request.type, success: true, data }) + '\n');
});
input.on('close', () => process.exit(0));
