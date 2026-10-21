import test from 'node:test';
import {readFileSync} from 'node:fs';
import assert from 'node:assert/strict';
import {Controller,safeState,library} from './controller.mjs';
import {buildMessage} from './vendor/core.js';
const program='$name = "Alex"\nroot = Stack([TextContent("Native output"), Form("reply", Buttons([Button("Prepare response")]), [Input("name", "Name", "text", {}, $name), Select("size", [SelectItem("Small", "Small"), SelectItem("Large", "Large")], "Size", {}, "Small"), Checkbox("confirmed", "Confirmed", false)])])';
test('official parser streams a typed native tree and preserves edited fields',()=>{
 let persisted;const c=new Controller(()=>{},state=>persisted=state);
 c.update({messageId:'a',blockId:0,source:program.slice(0,65),complete:false});
 c.update({messageId:'a',blockId:0,source:program,complete:true});
 assert.equal(c.error,'');assert.equal(c.root.typeName,'Stack');
 const form=c.root.props.children[1];const input=form.props.fields[0];
 c.field(input,'reply').setValue('Edited');
 c.update({messageId:'a',blockId:0,source:program.replace('Native output','Revised'),complete:true});
 assert.equal(c.field(c.root.props.children[1].props.fields[0],'reply').value,'Edited');
 assert.equal(persisted.$name,'Edited');assert.equal(c.submit('reply').type,'draft');
 c.dispose();
});
test('restores state from host without emitting an action or save',()=>{
 let saves=0;const c=new Controller(()=>{},()=>saves++);
 c.update({messageId:'restored',state:{$name:'Saved'},source:buildMessage({content:program,end:true}),complete:true});
 assert.equal(c.field(c.root.props.children[1].props.fields[0],'reply').value,'Saved');assert.equal(saves,0);assert.equal(c.actionId,0);c.dispose();
});
test('rejects executable tools and unsupported final components',()=>{
 const c=new Controller();c.update({source:'data = Query("exec", {})\nroot = Root([Text("fake data")])',complete:true});assert.match(c.error,/unavailable/);
 c.update({source:'root = Root([Browser("https://example.com")])',complete:true});assert.match(c.error,/Unknown|unknown|Browser/);c.dispose();
});
test('retains last tree during partial revisions and resets between messages',()=>{
 const c=new Controller();c.update({messageId:'a',source:program,complete:true});const root=c.root;
 c.update({messageId:'a',source:'root = Root([Input(',complete:false});assert.equal(c.error,'');assert.ok(c.root);
 c.update({messageId:'b',source:'root = Root([Text("Fresh")])',complete:true});assert.equal(c.store.get('$name'),undefined);assert.notEqual(c.root,root);c.dispose();
});
test('state serialization excludes secret fields and prototype keys',()=>{
 assert.deepEqual(safeState(JSON.parse('{"password":"x","token":"x","name":"ok","nested":{"secret":"x","ready":true},"__proto__":1}')),{name:'ok',nested:{ready:true}});
 assert.equal(library.root,'Stack');
});
test('submits untouched defaults, restores plain fields, and ignores host state echoes',()=>{
 const c=new Controller();
 c.update({messageId:'a',blockId:'one',source:program,state:{reply:{size:'Large',confirmed:true}},complete:true});
 const initial=c.submit('reply','Preferences');
 assert.match(initial.text,/"name": "Alex"/);assert.match(initial.text,/"size": "Large"/);assert.match(initial.text,/"confirmed": true/);
 const select=c.root.props.children[1].props.fields[1];c.field(select,'reply').setValue('Small');
 c.update({messageId:'a',blockId:'one',source:program,state:{reply:{size:'Large'}},complete:true});
 assert.match(c.submit('reply').text,/"size": "Small"/);
 c.update({messageId:'a',blockId:'two',source:program,complete:true});
 assert.equal(c.store.get('reply'),undefined);assert.match(c.submit('reply').text,/"size": "Small"/);c.dispose();
});
test('blocks drafts during streaming, errors, and sensitive-field output',()=>{
 const c=new Controller();c.update({source:program,complete:false});assert.equal(c.submit('reply'),null);
 c.update({source:'root = Root([Input("password", "Password")])',complete:true});assert.match(c.error,/Sensitive/);assert.equal(c.submit(),null);
 c.update({source:'x'.repeat(65537),complete:false});assert.match(c.error,/source limit/);c.dispose();
});
test('rejects sensitive aliases and invalid field/form/state names consistently',()=>{
 const names=['passwd','apikey','api_key','privatekey','private_key','PASSWORD','__proto__','constructor','prototype','','x'.repeat(65),'é'.repeat(33),'bad\nname','bad\u0085name'];
 for(const name of names){
  assert.deepEqual(safeState(Object.fromEntries([[name,'filtered']])),{},name);
  for(const component of ['Input','Form']){
   const c=new Controller();const args=component==='Form'?`${JSON.stringify(name)}, []`:`${JSON.stringify(name)}, "Label"`;
   c.update({source:`root = Root([${component}(${args})])`,complete:true});
   assert.match(c.error,/Sensitive|invalid/,`${component}: ${JSON.stringify(name)}`);assert.equal(c.submit(),null);c.dispose();
  }
 }
 const valid='é'.repeat(32);assert.deepEqual(safeState({[valid]:'retained'}),{[valid]:'retained'});
 const c=new Controller();c.update({source:'$api_key = "sensitive"\nroot = Root([Input("ordinary", "Label", $api_key)])',complete:true});assert.match(c.error,/Sensitive|invalid/);assert.equal(c.submit(),null);c.dispose();
});

test('required collection and numeric fields accept populated defaults',()=>{
 const c=new Controller();
 c.update({source:'root = Stack([Form("reply", Buttons([Button("Prepare")]), [Chips("choices", "multiple", [ChipItem("a", "A")], {required:true}, ["a"]), Slider("amount", "continuous", 0, 10, 1, [0], "Amount", {required:true}), CheckBoxGroup("checks", [CheckBoxItem("Ready", "", "ready", true)], {required:true})])])',complete:true});
 assert.equal(c.error,'');assert.ok(c.submit('reply'));assert.deepEqual(c.validationErrors,{});
 const chips=c.root.props.children[0].props.fields[0];c.field(chips,'reply').setValue([]);
 assert.equal(c.submit('reply'),null);assert.equal(c.validationErrors.choices,'This field is required.');
 c.field(chips,'reply').setValue(['a']);assert.ok(c.submit('reply'));c.dispose();
});

test('table values nested in forms survive serialization and restoration',()=>{
 const saved={reply:{table:[{id:'one',values:['kept',2]}]}};
 assert.deepEqual(safeState(saved),saved);
 const c=new Controller();c.update({source:'root = Stack([Form("reply", Buttons([Button("Prepare")]), [EditableTable("table", [{type:"text",key:"item",header:"Item"}], [{id:"one",values:["old"]}])])])',state:saved,complete:true});
 assert.equal(c.error,'');assert.match(c.submit('reply').text,/kept/);c.dispose();
});

test('nested card component props participate in the same validation walk',()=>{
 for(const source of [
  'root = Stack([ContextCardItem("one", Input("password", "Hidden password"))])',
  'root = Stack([CompositeCardItem("one", null, [], Input("password", "Hidden password"))])'
 ]){
  const c=new Controller();c.update({source,complete:true});assert.match(c.error,/Sensitive/);assert.equal(c.submit(),null);c.dispose();
 }
});

test('modal visibility reads and writes its reactive OpenUI binding',()=>{
 const c=new Controller();c.update({source:'$open = false\nroot = Stack([Modal("Dialog", $open, [TextContent("Body")])])',complete:true});
 assert.equal(c.error,'');let modal=c.root.props.children[0];assert.equal(c.binding(modal,'open',false,'modal').value,false);
 c.store.set('$open',true);modal=c.root.props.children[0];assert.equal(c.binding(modal,'open',false,'modal').value,true);
 c.binding(modal,'open',false,'modal').setValue(false);assert.equal(c.store.get('$open'),false);c.dispose();
});


test('form rules reach controls inside composite footer objects',()=>{
 const c=new Controller();
 c.update({source:'root = Stack([Form("reply", Buttons([Button("Prepare")]), [CompositeCardItem("one", null, [], {price:Input("nested", "Nested", "text", {required:true})})])])',complete:true});
 assert.equal(c.error,'');assert.equal(c.submit('reply'),null);assert.equal(c.validationErrors.nested,'This field is required.');
 c.field(c.root.props.children[0].props.fields[0].props.footer.price,'reply').setValue('ready');
 assert.ok(c.submit('reply'));c.dispose();
});


test('standard component fixtures parse with the pinned library schemas',()=>{
 const fixtures=JSON.parse(readFileSync(new URL('./standard-fixtures.json',import.meta.url),'utf8'));
 for(const fixture of fixtures){const c=new Controller();c.update({source:fixture.source,complete:true});assert.equal(c.error,'',fixture.name);assert.ok(c.root,fixture.name);c.dispose();}
});


test('state and drafts obey host UTF-8 byte bounds',()=>{
 assert.equal(new TextEncoder().encode(safeState('é'.repeat(2048))).length,2048);
 assert.equal(safeState('nul\0removed'),'nulremoved');
 const c=new Controller();c.update({source:'root = Stack([Input("name","Name","text")])',complete:true});
 c.field(c.root.props.children[0]).setValue('🧭'.repeat(2048));
 const action=c.submit(undefined,'🧭'.repeat(3000));assert.ok(new TextEncoder().encode(action.text).length<=8192);c.dispose();
});
