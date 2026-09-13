import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { balancePaneLayout, leaf, split } from '../src/lib/panes.ts';

function leafAreas(layout, area=1, result={}) {
  if(!('axis' in layout)) { result[layout.id]=area; return result; }
  leafAreas(layout.first,area*layout.ratio,result);
  leafAreas(layout.second,area*(1-layout.ratio),result);
  return result;
}
function assertEqualAreas(layout,expected) {
  for(const [id,area] of Object.entries(leafAreas(layout)))assert.ok(Math.abs(area-expected)<Number.EPSILON,`${id} should occupy ${expected}, got ${area}`);
}

const three=split('root','horizontal',leaf('one'),split('right','horizontal',leaf('two'),leaf('three'),.8),.8);
const balancedThree=balancePaneLayout(three);
assert.equal(balancedThree.ratio,1/3);
assert.equal(balancedThree.second.ratio,.5);
assertEqualAreas(balancedThree,1/3);

const four=split('root','horizontal',split('left','vertical',leaf('one'),leaf('two'),.2),split('right','vertical',leaf('three'),leaf('four'),.9),.7);
assertEqualAreas(balancePaneLayout(four),.25);
assert.equal(three.ratio,.8,'balancing must not mutate the saved source tree');

const surface=readFileSync(new URL('../src/lib/components/AppSurface.svelte',import.meta.url),'utf8');
assert.match(surface,/commandModifier && event\.altKey[^\n]+event\.code === 'Equal'/,'the direct balance shortcut must accept Cmd/Ctrl+Alt+=');
assert.match(surface,/id:'balance-panes',label:'Balance panes'/,'Controls must expose the balance action');
assert.match(surface,/if\(command\.kind==='equalize-panes'\) \{ balanceWorkspacePanes\(\);return; \}/,'Vim equalize must use the same balancing behavior');

console.log('pane balance tests passed');
