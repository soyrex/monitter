import { mount, signal, cleanup, batch, View, Text, Input } from "nsh";
import { nativeComponent } from "nsh/native";
import { Controller } from "./controller.mjs";

const Bridge = nativeComponent("monitter.openui.bridge", {props:["state","action","status","error","height"],events:["Update"]});
function App(){
 const theme=signal({dark:true,accent:"#86a9ff",paper:"#171b22",panel:"#202630",ink:"#e6e9ef",muted:"#a4aebd",line:"#384353",fontFamily:".AppleSystemUIFont",fontSize:14});
 const state=signal(null);const action=signal(null);const status=signal("ready");const error=signal("");const height=signal(100);
 const roots=signal([]);const records=new Map();
 let controller;
 function sync(){
  const live=new Set();
  function reconcile(node,path,form,parentType){
   const key=path+":"+(node.statementId||"")+":"+node.typeName;
   live.add(key);
   let rec=records.get(key);
   if(!rec){rec={key,type:node.typeName,props:signal(node.props,{equals:false}),children:signal([]),form};records.set(key,rec);}
   rec.form=form;rec.parentType=parentType;rec.props.set(node.props);
   const nextForm=node.typeName==="Form"?node.props.name:form;
   const children=(node.props.children||[]).filter(n=>n&&n.type==="element").map((child,i)=>reconcile(child,path+"/"+i,nextForm,node.typeName));
   rec.children.set(children);
   return rec;
  }
  batch(()=>{
   if(controller.root)roots.set([reconcile(controller.root,"root",undefined)]);
   else roots.set([]);
   for(const key of records.keys())if(!live.has(key))records.delete(key);
   error.set(controller.error);
   status.set(controller.error?"error":controller.root?"rendered":controller.complete?"error":"incomplete");
  });
 }
 controller=new Controller(sync,s=>state.set(s));
 cleanup(()=>controller.dispose());
 function update(value){
  batch(()=>{
   action.set(null);
   if(value.theme&&typeof value.theme==="object")theme.set({...theme(),...value.theme});
   controller.update(value);
  });
 }
 function submit(form,label){const payload=controller.submit(form,label);if(payload)action.set(payload);}
 const textStyle=()=>({minWidth:0,maxWidth:"100%",flexShrink:1,fontFamily:theme().fontFamily,fontSize:theme().fontSize,color:theme().ink});
 const mutedStyle=()=>({...textStyle(),fontSize:12,color:theme().muted});
 function button(label,onPress,selected=false){return <View role="button" accessibilityLabel={label} onPress={onPress} style={{padding:10,minHeight:36,borderRadius:7,borderWidth:1,borderColor:selected?theme().accent:theme().line,backgroundColor:selected?theme().panel:theme().paper,opacity:controller.complete?1:0.6}}><Text style={textStyle()}>{label}</Text></View>;}
 function Children(props){return <Each of={props.record.children()} key={r=>r.key}>{rec=><Node record={rec()}/>}</Each>;}
 function Node(props){
  const rec=props.record;
  const p=()=>rec.props();
  const field=()=>{p();return controller.field({typeName:rec.type,props:p()},rec.form);};
  const fieldValue=()=>field().value;
  const setField=value=>field().setValue(value);
  const box=()=>({gap:10,padding:0,minWidth:0,maxWidth:"100%",flexShrink:0});
  switch(rec.type){
   case "Root":case "Stack":return <View style={{...box(),width:"100%"}}><Children record={rec}/></View>;
   case "Row":return <View style={{...box(),flexDirection:"row",flexWrap:"wrap",alignItems:"stretch",width:"100%"}}><Children record={rec}/></View>;
   case "Card":return <View style={{...box(),...(rec.parentType==="Row"?{flexBasis:280,flexGrow:1,flexShrink:1}:{width:"100%"}),padding:14,borderRadius:10,borderWidth:1,borderColor:theme().line,backgroundColor:theme().panel}}><Text style={{...textStyle(),fontWeight:"600"}}>{p().title}</Text><Children record={rec}/></View>;
   case "Heading":return <Text style={{...textStyle(),fontSize:20,fontWeight:"600"}}>{p().content}</Text>;
   case "Text":return <Text selectable style={textStyle()}>{p().content}</Text>;
   case "Badge":return <View style={{alignSelf:"flex-start",paddingHorizontal:8,paddingVertical:4,borderRadius:8,backgroundColor:theme().panel,borderWidth:1,borderColor:theme().line}}><Text style={{...mutedStyle(),color:theme().accent}}>{p().label}</Text></View>;
   case "Form":return <View style={{...box(),padding:12,borderRadius:8,borderWidth:1,borderColor:theme().line}}><Children record={rec}/>{button(p().submitLabel||"Prepare response",()=>submit(p().name,p().submitLabel||"Form response"))}<Text style={mutedStyle()}>Review the response in your composer before sending.</Text></View>;
   case "Input":return <View style={{gap:5}}><Text style={textStyle()}>{p().label}</Text><Input accessibilityLabel={p().label} value={String(fieldValue()??"")} placeholder={p().placeholder||""} maxLength={2048} onChange={value=>setField(value)} style={{...textStyle(),height:38,padding:9,borderRadius:6,borderWidth:1,borderColor:theme().line,backgroundColor:theme().paper}}/></View>;
   case "Checkbox":return <View role="checkbox" accessibilityLabel={p().label} accessibilityState={{checked:!!fieldValue()}} onPress={()=>setField(!fieldValue())} style={{flexDirection:"row",gap:8,paddingVertical:7}}><Text style={{...textStyle(),color:theme().accent}}>{fieldValue()?"☑":"☐"}</Text><Text style={textStyle()}>{p().label}</Text></View>;
   case "Select":return <View style={{gap:6}}><Text style={textStyle()}>{p().label}</Text><View style={{flexDirection:"row",flexWrap:"wrap",gap:6}}><Each of={p().options||[]} key={option=>option}>{option=><View role="radio" accessibilityLabel={option()} accessibilityState={{checked:fieldValue()===option()}} onPress={()=>setField(option())} style={{padding:8,borderRadius:6,borderWidth:1,borderColor:fieldValue()===option()?theme().accent:theme().line,backgroundColor:theme().paper}}><Text style={textStyle()}>{option()}</Text></View>}</Each></View></View>;
   case "Button":return <View role="button" accessibilityLabel={p().label} onPress={()=>submit(rec.form,p().label)} style={{padding:10,minHeight:36,borderRadius:7,borderWidth:1,borderColor:theme().line,backgroundColor:theme().paper}}><Text style={textStyle()}>{p().label}</Text></View>;
   case "Table":return <View style={{gap:0,borderWidth:1,borderColor:theme().line,borderRadius:6}}><View style={{flexDirection:"row",backgroundColor:theme().panel}}><Each of={p().columns||[]}>{column=><Text style={{...textStyle(),fontWeight:"600",flex:1,padding:8}}>{column()}</Text>}</Each></View><Each of={p().rows||[]}>{row=><View style={{flexDirection:"row",borderTopWidth:1,borderColor:theme().line}}><Each of={row()}>{cell=><Text selectable style={{...textStyle(),flex:1,padding:8}}>{String(cell()??"")}</Text>}</Each></View>}</Each></View>;
   default:return <Text style={mutedStyle()}>Unsupported native component.</Text>;
  }
 }
 return <View style={{width:"100%",minWidth:0}}>
  <Bridge state={state()} action={action()} status={status()} error={error()} height={height()} onUpdate={update} style={{height:0,width:0}}/>
  <View style={{width:"100%",minWidth:0,padding:8,gap:8,flexShrink:0}} onLayout={event=>{const h=Math.max(80,Math.min(100000,Math.ceil(event.height)));if(Number.isFinite(h)&&Math.abs(h-height())>1)height.set(h);}}>
   <Each of={roots()} key={rec=>rec.key}>{rec=><Node record={rec()}/>}</Each>
  </View>
 </View>;
}
mount(App);
