import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source=readFileSync(new URL('../src/lib/components/AppSurface.svelte',import.meta.url),'utf8');
assert.match(source,/const startingTaskDraft = \$derived\(currentTaskDraft && currentDraftId && composerPending\[`draft:\$\{currentDraftId\}`\]/,'task creation needs a visible starting-chat state');
assert.match(source,/\{:else if startingTaskDraft\}[\s\S]*?<StartingTaskPane[^>]+messages=\{optimisticMessages\.filter\(message=>message\.kind==='draft'/,'the pending draft must render the starting chat shell with its optimistic transcript');
assert.match(source,/taskDrafts\[draftId\] = \{ \.\.\.draft, \.\.\.captured, title: values\.title \}/,'the real chat title must replace New chat immediately');

const pane=readFileSync(new URL('../src/lib/components/StartingTaskPane.svelte',import.meta.url),'utf8');
assert.match(pane,/<section class="starting-task-layout" aria-label="Starting chat">/,'the transition needs a proper chat region');
assert.match(pane,/<MessagePane[\s\S]*?<article class="message user optimistic-message"/,'the first message must be in the chat transcript');

console.log('new chat transition contract passed');
