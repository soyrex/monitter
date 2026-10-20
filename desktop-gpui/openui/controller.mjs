import {z,createLibrary,defineComponent,createStreamingParser,createStore,evaluateRoot,resolveStateField,markReactive,parseMessage} from './vendor/core.js';
const secret=/(password|secret|token|credential|authorization|cookie)/i;
const unsafeName=name=>secret.test(name)||['__proto__','constructor','prototype'].includes(name);
const reactive=s=>{markReactive(s);return s;};
const children=z.array(z.any());
const specs={
 Root:{children}, Stack:{children}, Row:{children},
 Card:{title:z.string(),children},
 Heading:{content:z.string()}, Text:{content:z.string()}, Badge:{label:z.string()},
 Form:{name:z.string(),children,submitLabel:z.string().optional()},
 Input:{name:z.string(),label:z.string(),value:reactive(z.string().optional()),placeholder:z.string().optional()},
 Select:{name:z.string(),label:z.string(),options:z.array(z.string()),value:reactive(z.string().optional())},
 Checkbox:{name:z.string(),label:z.string(),checked:reactive(z.boolean().optional())},
 Button:{label:z.string()},
 Table:{columns:z.array(z.string()),rows:z.array(z.array(z.union([z.string(),z.number(),z.boolean(),z.null()])))},
};
export const library=createLibrary({id:'monitter-native-v1',root:'Root',components:Object.entries(specs).map(([name,shape])=>defineComponent({name,description:`Native ${name}`,props:z.object(shape),component:name}))});
export function safeState(value,depth=0){
 if(depth>4)return undefined;
 if(value===null||typeof value==='boolean')return value;
 if(typeof value==='number')return Number.isFinite(value)?value:undefined;
 if(typeof value==='string')return value.slice(0,2048);
 if(Array.isArray(value))return value.slice(0,32).map(v=>safeState(v,depth+1)??null);
 if(value&&typeof value==='object')return Object.fromEntries(Object.entries(value).slice(0,64).filter(([k])=>!secret.test(k)&&!['__proto__','constructor','prototype'].includes(k)).map(([k,v])=>[k,safeState(v,depth+1)]).filter(([,v])=>v!==undefined));
 return undefined;
}
function boundedState(value){const state=safeState(value)||{};return JSON.stringify(state).length<=16384?state:{};}
export class Controller{
 constructor(onChange=()=>{},onState=()=>{}){
  this.store=createStore();this.parser=createStreamingParser(library.toJSONSchema(),library.root);this.result=null;this.root=null;this.error='';this.complete=false;this.identity=null;this.actionId=0;this.hydrating=false;
  this.onChange=onChange;this.onState=onState;
  this.unsubscribe=this.store.subscribe(()=>{this.evaluate();if(!this.hydrating)this.onState(boundedState(this.store.getSnapshot()));this.onChange();});
  this.context={getState:n=>this.store.get(n),resolveRef:()=>undefined};
 }
 update(update){
  const identity=String(update.messageId??'')+':'+String(update.blockId??'');
  if(this.identity!==identity){this.identity=identity;this.parser=createStreamingParser(library.toJSONSchema(),library.root);this.store.dispose();this.unsubscribe=this.store.subscribe(()=>{this.evaluate();if(!this.hydrating)this.onState(boundedState(this.store.getSnapshot()));this.onChange();});this.result=null;this.root=null;this.hydrating=true;this.store.initialize({},boundedState(update.state));this.hydrating=false;}
  this.complete=update.complete===true;this.error='';
  try{
   const source=String(update.source??'');
   if(source.length>65536)throw Error('OpenUI exceeds the native source limit.');
   const content=parseMessage(source,{streaming:!this.complete}).content;
   const result=this.parser.set(content,this.complete);
   if(result.queryStatements.length||result.mutationStatements.length)throw Error('Query and Mutation are unavailable in native output.');
   const errors=result.meta.errors.filter(e=>e.severity!=='warning');
   if(errors.length&&this.complete)throw Error(errors.map(e=>e.message).join('\n'));
   if((!errors.length||!this.root)&&(result.root||result.rootExpression)){this.result=result;this.hydrating=true;this.store.initialize(result.stateDeclarations,{});this.hydrating=false;this.evaluate();}
   if(!this.root&&this.complete)throw Error('OpenUI did not produce a supported Root component.');
  }catch(error){this.hydrating=false;if(this.complete||/unavailable|limit/.test(String(error)))this.error=String(error?.message||error).slice(0,2048);}
  this.onChange();
 }
 evaluate(){if(!this.result)return;const errors=[];try{const next=evaluateRoot(this.result,{ctx:this.context,library,store:this.store,errors});if(next){this.validate(next);this.root=next;}if(errors.length&&this.complete)this.error=errors.map(e=>e.message).join('\n').slice(0,2048);}catch(error){if(this.complete)this.error=String(error?.message||error).slice(0,2048);}}
 validate(node,depth=0,count={n:0}){
  if(depth>20||++count.n>300)throw Error('OpenUI exceeds the native component limit.');
  if(!specs[node.typeName])throw Error(`Unsupported component: ${node.typeName}`);
  if(node.props?.name&&unsafeName(node.props.name))throw Error('Sensitive fields are unavailable in native output.');
  if(node.typeName==='Table'&&(node.props.columns.length>8||node.props.rows.length>32||node.props.rows.some(row=>row.length>8)))throw Error('Native tables support up to 8 columns and 32 rows.');
  if(node.typeName==='Select'&&node.props.options.length>32)throw Error('Native selects support up to 32 options.');
  for(const child of node.props?.children||[]){if(child&&child.type==='element')this.validate(child,depth+1,count);}
 }
 field(node,form){const p=node.props;const key=node.typeName==='Checkbox'?'checked':'value';return resolveStateField(p.name,p[key],this.store,this.context,name=>form?this.store.get(form)?.[name]:this.store.get(name),(name,value)=>{if(unsafeName(name))return;const safe=safeState(value);if(form)this.store.set(form,{...(this.store.get(form)||{}),[name]:safe});else this.store.set(name,safe);});}
 submit(form,label='Prepare response'){
  if(!this.complete||this.error)return null;
  const state=boundedState(this.store.getSnapshot());
  const values={...(form?state[form]||{}:state)};
  const collect=(node,currentForm)=>{
   const scope=node.typeName==='Form'?node.props.name:currentForm;
   if(['Input','Select','Checkbox'].includes(node.typeName)&&scope===form){const value=this.field(node,scope).value;values[node.props.name]=safeState(value??(node.typeName==='Checkbox'?false:''));}
   for(const child of node.props.children||[])if(child?.type==='element')collect(child,scope);
  };
  if(this.root)collect(this.root,undefined);
  const text=`${label}\n${JSON.stringify(values,null,2)}`.slice(0,8192);
  return {type:'draft',text,id:String(++this.actionId)};
 }
 dispose(){this.unsubscribe();this.store.dispose();}
}
