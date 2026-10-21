import {z,createLibrary,defineComponent,createStreamingParser,createStore,evaluateRoot,resolveStateField,markReactive,parseMessage} from './vendor/core.js';
const secret=/(password|passwd|secret|token|credential|authorization|cookie|api_?key|private_?key)/i;
const unsafeName=name=>typeof name!=='string'||!name.length||name.length>64||new TextEncoder().encode(name).length>64||/[\u0000-\u001f\u007f-\u009f]/u.test(name)||secret.test(name)||['__proto__','constructor','prototype'].includes(name);
function editableTableFieldName(node){let hash=2166136261;for(const ch of String(node.statementId??JSON.stringify(node.props))){hash^=ch.charCodeAt(0);hash=Math.imul(hash,16777619);}return `openui-table-${(hash>>>0).toString(36)}`;}
function safePatternMatch(source,value){
 if(typeof source!=='string'||source.length>128||!source.startsWith('^')||!source.endsWith('$'))return null;
 let inClass=false,escaped=false,repetitions=0;
 for(let i=1;i<source.length-1;i++){
  const ch=source[i];
  if(escaped){if(/[1-9]/.test(ch))return null;escaped=false;continue;}
  if(ch==='\\'){escaped=true;continue;}
  if(inClass){if(ch===']')inClass=false;continue;}
  if(ch==='['){inClass=true;continue;}
  if(/[()|]/.test(ch)||ch==='^'||ch==='$')return null;
  if('*+?'.includes(ch)){if(++repetitions>1)return null;continue;}
  if(ch==='{'){
   const match=/^\{(\d+)(?:,(\d+))?\}/.exec(source.slice(i));if(!match||++repetitions>1)return null;
   const min=Number(match[1]),max=match[2]===undefined?min:Number(match[2]);if(max<min||max>256)return null;i+=match[0].length-1;
  }
 }
 if(escaped||inClass)return null;
 try{return new RegExp(source).test(value);}catch{return null;}
}
const reactive=s=>{markReactive(s);return s;};
const children=z.array(z.any());
const rules=z.object({required:z.boolean().optional(),email:z.boolean().optional(),url:z.boolean().optional(),numeric:z.boolean().optional(),min:z.number().optional(),max:z.number().optional(),minLength:z.number().optional(),maxLength:z.number().optional(),pattern:z.string().optional()}).optional();
const flex={direction:z.enum(['row','column']).optional(),gap:z.enum(['none','xs','s','m','l','xl','2xl']).optional(),align:z.enum(['start','center','end','stretch','baseline']).optional(),justify:z.enum(['start','center','end','between','around','evenly']).optional(),wrap:z.boolean().optional()};
const specs={
 // Keep the project root alias for stored legacy blocks. The canonical library root is Stack.
 Root:{children}, Stack:{children,...flex}, Row:{children},
 Card:{children,variant:z.enum(['card','sunk','clear']).optional(),...flex},
 CardHeader:{title:z.string().optional(),subtitle:z.string().optional()},
 Heading:{content:z.string()}, Text:{content:z.string()}, TextContent:{text:z.string(),size:z.enum(['small','default','large','small-heavy','large-heavy']).optional()}, Badge:{label:z.string()},
 Tag:{text:z.string(),icon:z.any().optional(),size:z.enum(['sm','md','lg']).optional(),variant:z.enum(['neutral','info','success','warning','danger']).optional()}, TagBlock:{tags:z.array(z.string()),size:z.enum(['sm','md','lg']).optional()}, Separator:{orientation:z.enum(['horizontal','vertical']).optional(),decorative:z.boolean().optional()},
 Callout:{variant:z.enum(['info','warning','error','success','neutral']),title:z.string(),description:z.string(),visible:reactive(z.boolean().optional())}, TextCallout:{variant:z.enum(['neutral','info','warning','success','danger']).optional(),title:z.string().optional(),description:z.string().optional()},
 ListBlock:{items:z.array(z.any()),variant:z.enum(['number','image']).optional(),size:z.enum(['default','small']).optional()}, ListItem:{title:z.string(),subtitle:z.string().optional(),image:z.object({src:z.string(),alt:z.string()}).optional(),actionLabel:z.string().optional(),action:z.any().optional()}, CodeBlock:{language:z.string(),codeString:z.string()},
 Tabs:{items:z.array(z.any())}, TabItem:{value:z.string(),trigger:z.string(),content:z.array(z.any())},
 Accordion:{items:z.array(z.any())}, AccordionItem:{value:z.string(),trigger:z.string(),content:z.array(z.any())},
 Steps:{items:z.array(z.any())}, StepsItem:{title:z.string(),details:z.string()}, Carousel:{children:z.array(z.array(z.any())),variant:z.enum(['card','sunk']).optional()}, Modal:{title:z.string(),open:reactive(z.boolean().optional()),children,size:z.enum(['sm','md','lg']).optional()},
 Form:{name:z.string(),buttons:z.any(),fields:z.array(z.any()).optional()},
 FormControl:{label:z.string(),input:z.any(),hint:z.string().optional()}, Label:{text:z.string()},
 Input:{name:z.string(),placeholder:z.string().optional(),type:z.enum(['text','email','password','number','url']).optional(),rules,value:reactive(z.string().optional())},
 TextArea:{name:z.string(),placeholder:z.string().optional(),rows:z.number().optional(),rules,value:reactive(z.string().optional())},
 Select:{name:z.string(),items:z.array(z.any()),placeholder:z.string().optional(),rules,value:reactive(z.string().optional()),size:z.enum(['small','medium','large']).optional()}, SelectItem:{value:z.string(),label:z.string()},
 CheckBoxGroup:{name:z.string(),items:z.array(z.any()),rules,value:reactive(z.record(z.string(),z.boolean()).optional())},CheckBoxItem:{label:z.string(),description:z.string(),name:z.string(),defaultChecked:z.boolean().optional()},
 RadioGroup:{name:z.string(),items:z.array(z.any()),defaultValue:z.string().optional(),rules,value:reactive(z.string().optional())},RadioItem:{label:z.string(),description:z.string(),value:z.string()},
 SwitchGroup:{name:z.string(),items:z.array(z.any()),variant:z.enum(['clear','card','sunk']).optional(),value:reactive(z.record(z.string(),z.boolean()).optional())},SwitchItem:{label:z.string().optional(),description:z.string().optional(),name:z.string(),defaultChecked:z.boolean().optional()}, DatePicker:{name:z.string(),mode:z.enum(['single','range']).optional(),rules,value:reactive(z.any().optional())},
 Slider:{name:z.string(),variant:z.enum(['continuous','discrete']),min:z.number(),max:z.number(),step:z.number().optional(),defaultValue:z.array(z.number()).optional(),label:z.string().optional(),rules,value:reactive(z.array(z.number()).optional())},
 Button:{label:z.string(),action:z.any().optional(),variant:z.enum(['primary','secondary','tertiary']).optional(),type:z.enum(['normal','destructive']).optional(),size:z.enum(['extra-small','small','medium','large']).optional()}, Buttons:{buttons:z.array(z.any()),direction:z.enum(['row','column']).optional()}, IconButton:{name:z.string(),icon:z.any(),action:z.any().optional(),variant:z.enum(['primary','secondary','tertiary']).optional(),size:z.enum(['extra-small','small','medium','large']).optional(),shape:z.enum(['square','circle']).optional()},
 FollowUpBlock:{items:z.array(z.any())}, FollowUpItem:{text:z.string()},
 Image:{alt:z.string(),src:z.string().optional()},ImageBlock:{src:z.string(),alt:z.string().optional()},ImageGallery:{images:z.array(z.object({src:z.string(),alt:z.string().optional(),details:z.string().optional()}))},
 EditableTable:{name:z.string().default(''),columns:z.array(z.object({type:z.enum(['text','number','date-single','select','url']),key:z.string().default('default'),header:z.string().default(''),width:z.number().optional(),options:z.array(z.object({value:z.string(),label:z.string()})).optional()})).default([]),data:z.array(z.object({id:z.string(),values:z.array(z.union([z.string(),z.number()]))})).default([])},
 Chips:{name:z.string(),type:z.enum(['single','multiple']).default('multiple'),items:z.array(z.any()).default([]),rules,defaultValue:z.union([z.string(),z.array(z.string())]).optional()},ChipItem:{value:z.string(),label:z.string(),icon:z.any().optional(),disabled:z.boolean().optional()},
 OptionCards:{name:z.string(),type:z.enum(['single','multiple']).default('single'),items:z.array(z.any()).default([]),rules,defaultValue:z.union([z.string(),z.array(z.string())]).optional()},OptionCard:{value:z.string(),title:z.string(),subtitle:z.string().optional(),topContent:z.any().optional(),disabled:z.boolean().optional()},
 EntityList:{rows:z.array(z.object({left:z.string(),right:z.string(),rightVariant:z.enum(['text','number']).default('text')})).default([]),size:z.enum(['small','default']).default('default'),header:z.object({left:z.string(),right:z.string(),rightVariant:z.enum(['text','number']).default('text')}).optional(),footer:z.object({left:z.string(),right:z.string(),rightVariant:z.enum(['text','number']).default('text')}).optional()},
 Text:{variant:z.enum(['text','number']).default('text'),value:z.string().optional(),content:z.string().optional(),subtext:z.string().optional(),subtextVariant:z.enum(['text','number','metric']).default('text'),size:z.enum(['xs','sm','md','lg']).default('sm')},BoldText:{variant:z.enum(['text','number']).default('text'),value:z.string(),subtext:z.string().optional(),subtextVariant:z.enum(['text','number','metric']).default('text'),size:z.enum(['xs','sm','md','lg']).default('sm')},
 IconText:{icon:z.any(),iconVariant:z.enum(['neutral','info','success','warning','danger','inverted','filled','soft']).default('neutral'),iconSize:z.enum(['xs','s','m','l','xl','sm','md','lg']).default('m'),title:z.string(),subtitle:z.string().optional(),bold:z.boolean().default(false),layout:z.enum(['horizontal','vertical']).default('horizontal')},
 ImageText:{src:z.string(),alt:z.string().optional(),title:z.string(),subtitle:z.string().optional(),bold:z.boolean().default(false),layout:z.enum(['horizontal','vertical']).default('horizontal'),imageSize:z.number().optional()},ImageTextLarge:{src:z.string(),alt:z.string().optional(),title:z.string(),subtitle:z.string().optional(),bold:z.boolean().default(false)},
 MetricIndicatorInline:{value:z.string(),subtext:z.string().optional(),trend:z.object({direction:z.enum(['up','down']),value:z.number()}).optional()},MetricIndicatorWithStrikethrough:{value:z.string(),subtext:z.string().optional(),previousValue:z.string().optional(),trend:z.object({direction:z.enum(['up','down']),value:z.number()}).optional()},
 SnippetCardBlock:{items:z.array(z.any()).min(2),layout:z.enum(['grid']).default('grid'),responsive:z.boolean().default(true),action:z.any().optional(),gap:z.union([z.number(),z.string()]).optional()},SnippetCardItem:{id:z.string().optional(),lhs:z.any(),rhs:z.any().optional()},
 OverviewCardBlock:{items:z.array(z.any()).min(2),layout:z.enum(['grid','carousel']).default('grid'),responsive:z.boolean().default(true),action:z.any().optional(),gap:z.union([z.number(),z.string()]).optional()},OverviewCardItem:{id:z.string().optional(),top:z.any(),bottom:z.any().optional()},
 ContextCardBlock:{items:z.array(z.any()).min(2),layout:z.enum(['grid','carousel']).default('grid'),responsive:z.boolean().default(true),action:z.any().optional(),gap:z.union([z.number(),z.string()]).optional()},ContextCardItem:{id:z.string().optional(),title:z.any(),body:z.string().optional(),bgColor:z.enum(['gray']).optional(),bgImageSrc:z.string().optional(),bgImageAlt:z.string().optional()},
 CompositeCardBlock:{items:z.array(z.any()).min(2),layout:z.enum(['grid','carousel']).default('grid'),responsive:z.boolean().default(true),action:z.any().optional(),gap:z.union([z.number(),z.string()]).optional()},CompositeCardItem:{id:z.string().optional(),header:z.any().optional(),body:z.array(z.any()).default([]),footer:z.any().optional()},
 VisualCardBlock:{items:z.array(z.any()).min(2),layout:z.enum(['grid','carousel']).default('grid'),responsive:z.boolean().default(true),action:z.any().optional(),gap:z.union([z.number(),z.string()]).optional()},VisualCardItem:{body:z.any(),id:z.string().optional(),bgImageSrc:z.string().optional(),tag:z.any().optional(),bgImageAlt:z.string().optional()},
 Table:{columns:z.array(z.any())}, Col:{label:z.string(),data:z.any(),type:z.enum(['string','number','action']).optional()},
 BarChart:{labels:z.array(z.string()),series:z.array(z.any()),variant:z.enum(['grouped','stacked']).optional(),xLabel:z.string().optional(),yLabel:z.string().optional(),height:z.number().optional()},
 LineChart:{labels:z.array(z.string()),series:z.array(z.any()),variant:z.enum(['linear','natural','step']).optional(),xLabel:z.string().optional(),yLabel:z.string().optional(),height:z.number().optional()},
 Series:{category:z.string(),values:z.array(z.number())}, AreaChart:{labels:z.array(z.string()),series:z.array(z.any()),variant:z.enum(['linear','natural','step']).optional(),xLabel:z.string().optional(),yLabel:z.string().optional(),height:z.number().optional()},
 HorizontalBarChart:{labels:z.array(z.string()),series:z.array(z.any()),variant:z.enum(['grouped','stacked']).optional(),xLabel:z.string().optional(),yLabel:z.string().optional()},RadarChart:{labels:z.array(z.string()),series:z.array(z.any())},PieChart:{labels:z.array(z.string()),values:z.array(z.number()),variant:z.enum(['pie','donut']).optional(),appearance:z.enum(['circular','semiCircular']).optional()},RadialChart:{labels:z.array(z.string()),values:z.array(z.number())},SingleStackedBarChart:{labels:z.array(z.string()),values:z.array(z.number())},Slice:{category:z.string(),value:z.number()},ScatterChart:{datasets:z.array(z.any()),xLabel:z.string().optional(),yLabel:z.string().optional()},ScatterSeries:{name:z.string(),points:z.array(z.any())},Point:{x:z.number(),y:z.number(),z:z.number().optional()},
 // These card block item schemas accept component references and are validated by the same allowlist walk.
 MarkDownRenderer:{textMarkdown:z.string(),variant:z.enum(['clear','card','sunk']).optional()}, InlineHeader:{heading:z.string(),description:z.string().optional()}, Icon:{name:z.string()},
 // Legacy aliases remain parseable so already stored response blocks keep rendering.
 Checkbox:{name:z.string(),label:z.string(),checked:reactive(z.boolean().optional())}, Switch:{name:z.string(),label:z.string(),checked:reactive(z.boolean().optional())},
};
export const library=createLibrary({id:'monitter-native-v1',root:'Stack',components:Object.entries(specs).map(([name,shape])=>defineComponent({name,description:`Native ${name}`,props:z.object(shape),component:name}))});
const utf8=value=>new TextEncoder().encode(value).length;
function utf8Prefix(value,limit){if(utf8(value)<=limit)return value;let result='',bytes=0;for(const ch of value){const next=utf8(ch);if(bytes+next>limit)break;bytes+=next;result+=ch;}return result;}
export function safeState(value,depth=0){
 if(depth>6)return undefined;
 if(value===null||typeof value==='boolean')return value;
 if(typeof value==='number')return Number.isFinite(value)?value:undefined;
 if(typeof value==='string')return utf8Prefix(value.replaceAll('\0',''),2048);
 if(Array.isArray(value))return value.slice(0,32).map(v=>safeState(v,depth+1)??null);
 if(value&&typeof value==='object')return Object.fromEntries(Object.entries(value).slice(0,64).filter(([k])=>!unsafeName(k)).map(([k,v])=>[k,safeState(v,depth+1)]).filter(([,v])=>v!==undefined));
 return undefined;
}
function boundedState(value){const state=safeState(value)||{};return utf8(JSON.stringify(state))<=16384?state:{};}
// Traverse only component-bearing props; share the same walk with validation,
// draft collection and native reconciliation so nested controls retain scope.
export function componentChildren(props){
 const nested=['children','items','columns','fields','buttons','content','series','tags','data','body'].flatMap(key=>Array.isArray(props?.[key])?props[key].flat(Infinity):[]);
 for(const key of ['icon','topContent','lhs','rhs','top','bottom','header','tag','body','title','footer','input','buttons'])if(props?.[key]?.type==='element')nested.push(props[key]);
 for(const key of ['button','price'])if(props?.footer?.[key]?.type==='element')nested.push(props.footer[key]);
 return nested.filter(child=>child?.type==='element');
}
const standardSchema=library.toJSONSchema();
export class Controller{
 constructor(onChange=()=>{},onState=()=>{}){
  this.store=createStore();this.parser=createStreamingParser(standardSchema,library.root);this.result=null;this.root=null;this.error='';this.complete=false;this.identity=null;this.actionId=0;this.hydrating=false;this.validationErrors={};
  this.onChange=onChange;this.onState=onState;
  this.unsubscribe=this.store.subscribe(()=>{this.evaluate();if(!this.hydrating)this.onState(boundedState(this.store.getSnapshot()));this.onChange();});
  this.context={getState:n=>this.store.get(n),resolveRef:()=>undefined};
 }
 update(update){
  const identity=String(update.messageId??'')+':'+String(update.blockId??'');
  if(this.identity!==identity){this.identity=identity;this.parser=createStreamingParser(standardSchema,library.root);this.store.dispose();this.unsubscribe=this.store.subscribe(()=>{this.evaluate();if(!this.hydrating)this.onState(boundedState(this.store.getSnapshot()));this.onChange();});this.result=null;this.root=null;this.validationErrors={};this.hydrating=true;this.store.initialize({},boundedState(update.state));this.hydrating=false;}
  this.complete=update.complete===true;this.error='';
  try{
   const source=String(update.source??'');
   if(source.length>65536)throw Error('OpenUI exceeds the native source limit.');
   const content=parseMessage(source,{streaming:!this.complete}).content;
   const result=this.parser.set(content,this.complete);
   if(result.queryStatements.length||result.mutationStatements.length)throw Error('Query and Mutation are unavailable in native output.');
   if(Object.keys(result.stateDeclarations).some(unsafeName))throw Error('Sensitive or invalid state names are unavailable in native output.');
   const errors=result.meta.errors.filter(e=>e.severity!=='warning');
   if(errors.length&&this.complete)throw Error(errors.map(e=>e.message).join('\n'));
   if((!errors.length||!this.root)&&(result.root||result.rootExpression)){this.result=result;this.hydrating=true;this.store.initialize(result.stateDeclarations,{});this.hydrating=false;this.evaluate();}
   if(!this.root&&this.complete&&!this.error)throw Error('OpenUI did not produce a supported Root component.');
  }catch(error){this.hydrating=false;if(this.complete||/unavailable|limit/.test(String(error)))this.error=String(error?.message||error).slice(0,2048);}
  this.onChange();
 }
 evaluate(){if(!this.result)return;const errors=[];try{const next=evaluateRoot(this.result,{ctx:this.context,library,store:this.store,errors});if(next){this.validate(next);this.root=next;}if(errors.length&&this.complete)this.error=errors.map(e=>e.message).join('\n').slice(0,2048);}catch(error){if(this.complete)this.error=String(error?.message||error).slice(0,2048);}}
 validate(node,depth=0,count={n:0}){
  if(depth>20||++count.n>300)throw Error('OpenUI exceeds the native component limit.');
  if(!specs[node.typeName])throw Error(`Unsupported component: ${node.typeName}`);
  if(node.props?.name!==undefined&&unsafeName(node.props.name)&&!(node.typeName==='EditableTable'&&node.props.name===''))throw Error('Sensitive or invalid form/field names are unavailable in native output.');
  if(node.typeName==='Text'&&node.props.value===undefined&&node.props.content===undefined)throw Error('Text requires a value.');
  if(node.typeName==='Input'&&String(node.props.type||'').toLowerCase()==='password')throw Error('Password inputs are unavailable in native output.');
  if(node.typeName==='Table'&&node.props.columns.length>8)throw Error('Native tables support up to 8 columns.');
  if(node.typeName==='Select'&&node.props.items.length>32)throw Error('Native selects support up to 32 options.');
  if(node.typeName==='Col'&&Array.isArray(node.props.data)&&node.props.data.length>32)throw Error('Native table columns support up to 32 rows.');
  if(['Chips','OptionCards'].includes(node.typeName)&&node.props.items.length>32)throw Error(`Native ${node.typeName} supports up to 32 choices.`);
  if(node.typeName==='EditableTable'&&(node.props.columns.length>8||node.props.data.length>32||node.props.data.some(row=>row.values.length>8)))throw Error('Native editable tables support up to 8 columns and 32 rows.');
  if(node.typeName==='ImageGallery'&&node.props.images.length>32)throw Error('Native image galleries support up to 32 entries.');
  if(['PieChart','RadialChart','SingleStackedBarChart'].includes(node.typeName)&&(node.props.labels.length>32||node.props.values.length>32))throw Error('Native charts support up to 32 categories.');
  if(node.typeName==='ScatterChart'&&(node.props.datasets.length>8||node.props.datasets.some(dataset=>(dataset?.type==='element'?dataset.props.points:dataset?.points)?.length>32)))throw Error('Native scatter charts support up to 8 datasets and 32 points each.');
  if(node.typeName==='EntityList'&&node.props.size==='small'&&(node.props.header||node.props.footer))throw Error('Small entity lists do not support header or footer rows.');
  for(const key of ['items','buttons','tags'])if(Array.isArray(node.props?.[key])&&node.props[key].length>32)throw Error(`Native ${node.typeName} supports up to 32 ${key}.`);
  if(['BarChart','LineChart','AreaChart'].includes(node.typeName)&&(node.props.labels.length>32||node.props.series.length>8||node.props.series.some(s=>(s?.type==='element'?s.props.values:s?.values)?.length>32)))throw Error('Native charts support up to 8 series and 32 data points.');
  for(const child of componentChildren(node.props))this.validate(child,depth+1,count);
 }
 field(node,form){const p=node.props;const name=node.typeName==='EditableTable'&&!p.name?editableTableFieldName(node):p.name;const key=['Checkbox','Switch'].includes(node.typeName)?'checked':'value';const legacyInput=node.typeName==='Input'&&p.value===undefined&&!['text','email','number','search','tel','url','date'].includes(String(p.type||'').toLowerCase());const choiceDefault=['CheckBoxGroup','SwitchGroup'].includes(node.typeName)?Object.fromEntries((p.items||[]).filter(item=>item?.type==='element').map(item=>[item.props.name,!!item.props.defaultChecked])):undefined;const initial=node.typeName==='Select'&&p.value===undefined&&typeof p.rules==='string'?p.rules:legacyInput?p.type:p[key]??p.defaultValue??choiceDefault;return resolveStateField(name,initial,this.store,this.context,field=>form?this.store.get(form)?.[field]:this.store.get(field),(field,value)=>{if(unsafeName(field))return;delete this.validationErrors[field];const safe=safeState(value);if(form)this.store.set(form,{...(this.store.get(form)||{}),[field]:safe});else this.store.set(field,safe);});}
 binding(node,property,fallback,identity){const name=editableTableFieldName({...node,statementId:identity})+"-"+property;return resolveStateField(name,node.props[property]??fallback,this.store,this.context,key=>this.store.get(key),(key,value)=>{if(!unsafeName(key))this.store.set(key,safeState(value));});}
 fieldError(node){const name=node.typeName==='EditableTable'&&!node.props.name?editableTableFieldName(node):node.props.name;return this.validationErrors[name];}
 validateForm(form){const fields=new Set(['Input','TextArea','Select','DatePicker','Slider','CheckBoxGroup','RadioGroup','Chips','OptionCards']);const next={};const seen=new Set();const visit=(node,currentForm)=>{if(!node||node.type!=='element'||seen.has(node))return;seen.add(node);const p=node.props||{};if(node.typeName==='Form')currentForm=p.name;if(currentForm===form&&fields.has(node.typeName)&&p.rules){const name=p.name;const value=this.field(node,form).value;const text=typeof value==='string'?value.trim():'';const empty=value==null||(typeof value==='string'&&text==='')||(Array.isArray(value)&&value.length===0)||(value&&typeof value==='object'&&!Array.isArray(value)&&!Object.values(value).some(Boolean));const rule=p.rules;let message='';if(rule.required&&empty)message='This field is required.';else if(!empty&&rule.email&&!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(text))message='Enter a valid email address.';else if(!empty&&rule.url){try{const url=new URL(text);if(!['http:','https:'].includes(url.protocol))throw Error();}catch{message='Enter a valid URL.';}}else if(!empty&&rule.numeric&&!Number.isFinite(Number(value)))message='Enter a number.';else if(!empty&&typeof value==='string'&&rule.minLength!==undefined&&value.length<rule.minLength)message=`Use at least ${rule.minLength} characters.`;else if(!empty&&typeof value==='string'&&rule.maxLength!==undefined&&value.length>rule.maxLength)message=`Use no more than ${rule.maxLength} characters.`;else if(!empty&&rule.min!==undefined&&Number.isFinite(Number(value))&&Number(value)<rule.min)message=`Enter a value of at least ${rule.min}.`;else if(!empty&&rule.max!==undefined&&Number.isFinite(Number(value))&&Number(value)>rule.max)message=`Enter a value no greater than ${rule.max}.`;else if(!empty&&rule.pattern){const matched=safePatternMatch(rule.pattern,text);message=matched===null?'Pattern must be an anchored, simple expression with at most one repetition in the native renderer.':matched?'':'Value does not match the required pattern.';}if(message)next[name]=message;}
   for(const child of componentChildren(p))visit(child,currentForm);
  };visit(this.root,undefined);this.validationErrors=next;return Object.keys(next).length===0;}
 submit(form,label='Prepare response'){
  if(!this.complete||this.error)return null;
  if(form&&!this.validateForm(form)){this.onChange();return null;}
  const state=boundedState(this.store.getSnapshot());
  const values={...(form?state[form]||{}:state)};
  const collect=(node,currentForm)=>{
   const scope=node.typeName==='Form'?node.props.name:currentForm;
   if(['Input','TextArea','Select','Checkbox','CheckBoxGroup','RadioGroup','Switch','SwitchGroup','DatePicker','Slider','Chips','OptionCards','EditableTable'].includes(node.typeName)&&scope===form){const fieldNode=node.typeName==='EditableTable'?{...node,props:{...node.props,defaultValue:node.props.data}}:node;const value=this.field(fieldNode,scope).value;const outputName=node.typeName==='EditableTable'&&!node.props.name?editableTableFieldName(node):node.props.name;values[outputName]=safeState(value??(['Checkbox','Switch'].includes(node.typeName)?false:['CheckBoxGroup','SwitchGroup'].includes(node.typeName)?{}:['Chips','OptionCards'].includes(node.typeName)?(node.props.defaultValue??(node.props.type==='multiple'?[]:'')):node.typeName==='EditableTable'?node.props.data:''));}
   for(const child of componentChildren(node.props))collect(child,scope);
  };
  if(this.root)collect(this.root,undefined);
  const text=utf8Prefix(`${label}\n${JSON.stringify(values,null,2)}`,8192);
  return {type:'draft',text,id:String(++this.actionId)};
 }
 dispose(){this.unsubscribe();this.store.dispose();}
}
