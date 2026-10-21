import test from 'node:test';
import assert from 'node:assert/strict';
import { drawChart, normalizeChart, valueDomain, stackSegments, chartPalette, CHART_LIMITS } from './charts.mjs';

const methods = new Set(['save','restore','beginPath','moveTo','lineTo','bezierCurveTo','arc','rect','closePath','fill','stroke','clip','fillRect','fillText','translate','rotate']);
const properties = new Set(['fillStyle','strokeStyle','lineWidth','font','textAlign','textBaseline','globalAlpha']);
// NSH's limit counts flat encoded values, including opcodes, default arguments
// and property writes (nsh-js/src/canvas.rs record/record_method/send).
const encodedArity={save:0,restore:0,beginPath:0,moveTo:2,lineTo:2,bezierCurveTo:6,arc:6,rect:4,closePath:0,fill:2,stroke:1,clip:2,fillRect:4,fillText:4,translate:2,rotate:1};
const recordedValues = output => output.writes.length * 2 + output.calls.reduce((sum,c)=>sum+1+encodedArity[c.method],0);
function recorder() {
  const calls = [], writes = [], state = {};
  const ctx = new Proxy({}, {
    get(_, key) {
      assert.ok(methods.has(key), `Unsupported canvas method: ${String(key)}`);
      return (...args) => { calls.push({method:key,args,style:{...state}}); };
    },
    set(_, key, value) { assert.ok(properties.has(key), `Unsupported canvas property: ${String(key)}`); writes.push({key,value}); state[key] = value; return true; },
  });
  return {ctx,calls,writes};
}
function draw(type, props, size = {width:640,height:340}, theme) {
  const output = recorder(); drawChart(output.ctx, size, type, props, theme); return output;
}
function finiteDrawing(output) {
  for (const call of output.calls) {
    for (const arg of call.args) if (typeof arg === 'number') assert.ok(Number.isFinite(arg), `${call.method}: ${call.args}`);
    if (call.method === 'arc') assert.ok(call.args[2] >= 0);
    if (call.method === 'fillRect' || call.method === 'rect') assert.ok(call.args[2] >= 0 && call.args[3] >= 0);
  }
  for (const {value} of output.writes) if (typeof value === 'number') assert.ok(Number.isFinite(value));
  assert.equal(output.calls.filter(c => c.method === 'save').length, output.calls.filter(c => c.method === 'restore').length);
}
const element = props => ({type:'element', props});
const series = {labels:['A','B','C'],series:[{category:'First',values:[8,-4,2]},{category:'Second',values:[-2,6,3]}]};

test('normalizes official element series and points exactly like plain objects', () => {
  assert.deepEqual(normalizeChart('BarChart', element({...series,series:series.series.map(element)})), normalizeChart('BarChart',series));
  const datasets = [{name:'Sample',points:[{x:-2,y:5,z:12}]}];
  assert.deepEqual(normalizeChart('ScatterChart', {datasets:datasets.map(d => element({...d,points:d.points.map(element)}))}), normalizeChart('ScatterChart',{datasets}));
});
test('caps collections, pads ragged series, and sanitizes nonfinite values', () => {
  const p = normalizeChart('LineChart',{labels:['Known'],series:Array.from({length:20}, () => ({values:[NaN,Infinity,-Infinity,...Array(100).fill(3)]}))});
  assert.equal(p.series.length,CHART_LIMITS.series); assert.equal(p.labels.length,CHART_LIMITS.points); assert.equal(p.series[0].values.length,32);
  assert.deepEqual(p.series[0].values.slice(0,3),[0,0,0]); assert.equal(p.labels[0],'Known');
  assert.deepEqual(normalizeChart('BarChart',{labels:['A','B'],series:[{values:[5]}]}).series[0].values,[5,0]);
  const scatter = normalizeChart('ScatterChart',{datasets:Array.from({length:20},()=>({points:Array(100).fill({x:1,y:2})}))});
  assert.equal(scatter.datasets.length,8); assert.equal(scatter.datasets[0].points.length,32);
});
test('domains and signed stacks preserve separate positive and negative totals', () => {
  assert.deepEqual(valueDomain([]),[0,1]); assert.deepEqual(valueDomain([0,0]),[0,1]); assert.deepEqual(valueDomain([-3,8]),[-3,8]);
  const result = stackSegments([{values:[4,-3]},{values:[-2,-5]},{values:[6,8]}],2);
  assert.deepEqual(result.map(s => s[0]),[{from:0,to:4,value:4},{from:0,to:-2,value:-2},{from:4,to:10,value:6}]);
  assert.deepEqual(result.map(s => s[1]),[{from:0,to:-3,value:-3},{from:-3,to:-8,value:-5},{from:0,to:8,value:8}]);
  assert.deepEqual(valueDomain([8e100,-8e100]),[-8e100,8e100]);
});
test('palette follows current theme accent and remains deterministic', () => {
  const a = chartPalette('#8644aa',8), b = chartPalette('#16bb77',8);
  assert.equal(a[0],'#8644aa'); assert.equal(new Set(a).size,8); assert.deepEqual(a,chartPalette('#8644aa',8)); assert.notDeepEqual(a,b);
  const output=draw('BarChart',series,undefined,{accent:'#16bb77'});
  assert.ok(output.calls.some(c => c.method==='fillRect' && c.style.fillStyle==='#16bb77'));
});
test('grouped bars separate series and stacked bars share category positions', () => {
  for (const type of ['BarChart','HorizontalBarChart']) {
    const grouped = draw(type,{...series,variant:'grouped'}), stacked = draw(type,{...series,variant:'stacked'});
    const data = output => output.calls.filter(c => c.method==='fillRect').slice(2);
    assert.equal(data(grouped).length,6); assert.equal(data(stacked).length,6);
    const coordinate = type==='BarChart'?0:1;
    assert.notEqual(data(grouped)[0].args[coordinate],data(grouped)[3].args[coordinate]);
    assert.equal(data(stacked)[0].args[coordinate],data(stacked)[3].args[coordinate]);
    finiteDrawing(grouped); finiteDrawing(stacked);
  }
});
test('lines have real interpolation geometry and areas close to a signed zero baseline', () => {
  for (const type of ['LineChart','AreaChart']) for (const variant of ['linear','step','natural']) {
    const output=draw(type,{...series,variant}); finiteDrawing(output);
    assert.ok(output.calls.some(c => c.method==='stroke' && c.style.strokeStyle==='#86a9ff'));
    if (variant==='natural') assert.ok(output.calls.some(c=>c.method==='bezierCurveTo'));
    if (type==='AreaChart') assert.ok(output.calls.some(c=>c.method==='closePath'));
  }
  assert.ok(draw('LineChart',{...series,variant:'step'}).calls.filter(c=>c.method==='lineTo').length > draw('LineChart',{...series,variant:'linear'}).calls.filter(c=>c.method==='lineTo').length);
});
test('pie/donut and semicircle produce bounded true arcs with a reverse inner arc for holes', () => {
  for (const appearance of ['circular','semiCircular']) for (const variant of ['pie','donut']) {
    const output=draw('PieChart',{labels:['A','B','C'],values:[2,-3,5],appearance,variant}); finiteDrawing(output);
    const arcs=output.calls.filter(c=>c.method==='arc');
    assert.equal(arcs.length,variant==='donut'?6:3);
    const sweep=arcs.filter(c=>!c.args[5]).reduce((sum,c)=>sum+c.args[4]-c.args[3],0);
    assert.ok(Math.abs(sweep-(appearance==='semiCircular'?Math.PI:Math.PI*2))<1e-10);
    assert.equal(arcs.filter(c=>c.args[5]).length,variant==='donut'?3:0);
  }
});
test('radial chart draws concentric tracks and proportional value arcs', () => {
  const output=draw('RadialChart',{labels:['A','B'],values:[10,5]}); finiteDrawing(output);
  const arcs=output.calls.filter(c=>c.method==='arc'); assert.equal(arcs.length,4); assert.ok(arcs[0].args[2]>arcs[2].args[2]);
  assert.ok(Math.abs((arcs[1].args[4]-arcs[1].args[3])/(arcs[3].args[4]-arcs[3].args[3])-2)<1e-10);
});
test('radar renders polygon grids and distinct filled series with a labeled signed scale', () => {
  const output=draw('RadarChart',series); finiteDrawing(output);
  assert.equal(output.calls.filter(c=>c.method==='fill').length,2);
  assert.equal(output.calls.filter(c=>c.method==='closePath').length,6);
  assert.ok(output.calls.some(c=>c.method==='fillText'&&c.args[0]==='-4'));
});
test('scatter positions signed points on independent axes and scales z bubble radii', () => {
  const output=draw('ScatterChart',{datasets:[element({name:'Bubbles',points:[element({x:-3,y:-5,z:1}),{x:9,y:7,z:100}]})],xLabel:'Longitude',yLabel:'Latitude'}); finiteDrawing(output);
  const arcs=output.calls.filter(c=>c.method==='arc'); assert.equal(arcs.length,2);
  assert.ok(arcs[0].args[0]<arcs[1].args[0]); assert.ok(arcs[0].args[1]>arcs[1].args[1]); assert.ok(arcs[0].args[2]<arcs[1].args[2]);
  assert.ok(output.calls.some(c=>c.method==='fillText'&&c.args[0]==='Longitude'));
});
test('single stacked bar diverges left/right at the zero marker', () => {
  const output=draw('SingleStackedBarChart',{labels:['A','B','C'],values:[4,-2,6]}); finiteDrawing(output);
  const rects=output.calls.filter(c=>c.method==='fillRect').slice(4);
  assert.equal(rects.length,3); assert.ok(rects[1].args[0]<rects[0].args[0]); assert.ok(rects[2].args[0]>rects[0].args[0]);
});
test('every chart handles empty, zero, ragged, huge and one-point input with finite canvas operations', () => {
  const types=['BarChart','HorizontalBarChart','LineChart','AreaChart','PieChart','RadialChart','SingleStackedBarChart','RadarChart','ScatterChart'];
  for (const type of types) for (const value of [0,Number.MAX_VALUE,-Number.MAX_VALUE,NaN]) {
    finiteDrawing(draw(type,{}));
    const p={labels:['Only'],values:[value],series:[{category:'One',values:[value]}],datasets:[{name:'One',points:[{x:value,y:value,z:value}]}]};
    finiteDrawing(draw(type,p));
  }
  assert.equal(draw('LineChart',series,{width:NaN,height:Infinity}).calls.length,0);
});
test('maximum charts stay below a bounded native 8192 encoded-value budget', t => {
  const labels=Array.from({length:100},(_,i)=>`Category ${i}`),values=labels.map((_,i)=>(i%2?-1:1)*(i+1));
  const maximum={labels,values,series:Array.from({length:20},(_,i)=>({category:`Series ${i}`,values})),datasets:Array.from({length:20},(_,i)=>({name:`Points ${i}`,points:labels.map((_,j)=>({x:j-16,y:values[j],z:j}))}))};
  let maximumValues=0, maximumMethods=0, maximumType='';
  for (const type of ['BarChart','HorizontalBarChart','LineChart','AreaChart','PieChart','RadialChart','SingleStackedBarChart','RadarChart','ScatterChart']) {
    for (const variant of ['stacked','grouped','linear','natural','step','donut']) {
      const output=draw(type,{...maximum,variant}); finiteDrawing(output);
      const values=recordedValues(output);
      assert.ok(values<=8192,`${type}/${variant}: ${values} encoded values`);
      maximumMethods=Math.max(maximumMethods,output.calls.length);
      if(values>maximumValues){maximumValues=values;maximumType=`${type}/${variant}`;}
    }
  }
  t.diagnostic(`Worst case: ${maximumType}, ${maximumValues} encoded values; maximum ${maximumMethods} canvas method calls.`);
});
