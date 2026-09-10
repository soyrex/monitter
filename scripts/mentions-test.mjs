import assert from 'node:assert/strict';
import {mentionParts,mentionedAgentIds} from '../src/lib/mentions.ts';
const agents=[{id:'r',name:'Rafa'},{id:'j',name:'Justine'}];
assert.deepEqual(mentionedAgentIds('@rafa and @JUSTINE, @rafa',agents),['r','j']);
assert.deepEqual(mentionedAgentIds('email@rafa.com @rafael @unknown',agents),[]);
assert.equal(mentionParts('Hello @rafa\n@Justine!',agents).map(p=>p.text).join(''),'Hello @rafa\n@Justine!');
assert.deepEqual(mentionedAgentIds('@sam', [{id:'a',name:'Sam A'},{id:'b',name:'Sam B'}]),[]);
assert.deepEqual(mentionedAgentIds('@Sam-B', [{id:'a',name:'Sam A'},{id:'b',name:'Sam B'}]),['b']);
console.log('Mention matching tests passed');
