import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { balancePaneLayout, createPanePresetLayout, leaf, paneIds, panePresetDimensions, split } from '../src/lib/panes.ts';

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

for(const [mode,columns,rows] of [['single',1,1],['columns',2,1],['columns-3',3,1],['columns-4',4,1],['grid',2,2],['grid-3x2',3,2],['grid-4x2',4,2]]) {
  const ids=Array.from({length:columns*rows},(_,index)=>`pane-${index}`);
  let splitNumber=0;
  const layout=createPanePresetLayout(mode,ids,()=>`split-${++splitNumber}`);
  assert.deepEqual(panePresetDimensions(mode),[columns,rows]);
  assert.deepEqual(paneIds(layout),ids);
  assertEqualAreas(layout,1/ids.length);
  const positions=[];
  const visit=(node,x=0,y=0,width=1,height=1)=>{
    if(!('axis' in node)){positions.push({x,y});return;}
    if(node.axis==='horizontal'){
      visit(node.first,x,y,width*node.ratio,height);
      visit(node.second,x+width*node.ratio,y,width*(1-node.ratio),height);
    }else{
      visit(node.first,x,y,width,height*node.ratio);
      visit(node.second,x,y+height*node.ratio,width,height*(1-node.ratio));
    }
  };
  visit(layout);
  assert.equal(new Set(positions.map(position=>position.x.toFixed(6))).size,columns);
  assert.equal(new Set(positions.map(position=>position.y.toFixed(6))).size,rows);
}

const surface=readFileSync(new URL('../src/lib/components/AppSurface.svelte',import.meta.url),'utf8');
assert.match(surface,/commandModifier && event\.altKey[^\n]+event\.code === 'Equal'/,'the direct balance shortcut must accept Cmd/Ctrl+Alt+=');
assert.match(surface,/id:'balance-panes',label:'Balance panes'/,'Controls must expose the balance action');
assert.match(surface,/if\(command\.kind==='equalize-panes'\) \{ balanceWorkspacePanes\(\);return; \}/,'Vim equalize must use the same balancing behavior');

console.log('pane balance tests passed');
