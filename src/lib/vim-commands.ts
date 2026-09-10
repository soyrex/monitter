/** Commands Monitter can faithfully map to its pane tree and ordered tabs. */
export type VimTabTarget = { kind: 'index'; index: number } | { kind: 'relative'; offset: number } | { kind: 'last' };
export type VimCommand =
  | { kind: 'tabnew'; after?: VimTabTarget }
  | { kind: 'split'; axis: 'horizontal' | 'vertical'; size?: number }
  | { kind: 'terminal' }
  | { kind: 'tabselect'; target: VimTabTarget }
  | { kind: 'tabclose'; target?: VimTabTarget }
  | { kind: 'tabonly'; target?: VimTabTarget }
  | { kind: 'tabmove'; target: 'first' | 'last' | { kind: 'relative'; offset: number } | { kind: 'after'; index: number } }
  | { kind: 'close-pane'; target?: number } | { kind: 'only-pane' }
  | { kind: 'focus-pane'; target: 'first' | 'last' | 'next' | 'previous' | { kind: 'direction'; direction: 'left' | 'right' | 'up' | 'down' } | { kind: 'index'; index: number } }
  | { kind: 'rotate-panes'; direction: 1 | -1 } | { kind: 'exchange-pane'; target?: number }
  | { kind: 'move-pane'; edge: 'top' | 'bottom' | 'left' | 'right' }
  | { kind: 'equalize-panes'; axis?: 'horizontal' | 'vertical' }
  | { kind: 'resize-pane'; axis: 'horizontal' | 'vertical'; delta?: number; size?: number; maximize?: boolean }
  | { kind: 'help' };
export type VimCommandParse = { command: VimCommand } | { error: string };
export type VimCompletion = { value: string; label: string; detail: string };

const specs = [
  ['tabnew','new chat tab'],['tabnext','select next tab'],['tabprevious','select previous tab'],['tabfirst','select first tab'],['tablast','select last tab'],['tabclose','close a tab'],['tabonly','close all other tabs in this pane'],['tabmove','reorder current tab'],['split','split pane above/below'],['vsplit','split pane left/right'],['close','remove current pane safely'],['only','keep only current pane'],['resize','resize current pane'],['vertical resize','resize current pane width'],['wincmd h','focus pane left (Ctrl-W h)'],['wincmd j','focus pane below (Ctrl-W j)'],['wincmd k','focus pane above (Ctrl-W k)'],['wincmd l','focus pane right (Ctrl-W l)'],['wincmd =','equalize panes'],['wincmd r','rotate panes'],['wincmd x','exchange panes'],['terminal','open terminal tab'],['help','show Monitter command help'],
] as const;
const positive = (s:string) => /^\d+$/.test(s) && Number(s)>0 ? Number(s) : null;
const relative = (s:string) => /^[+-]\d*$/.test(s) ? s==='+'?1:s==='-'?-1:Number(s) : null;
function target(s:string):VimTabTarget|null { if(s==='$')return {kind:'last'}; const r=relative(s); if(r!==null)return {kind:'relative',offset:r}; const n=positive(s); return n?{kind:'index',index:n}:null; }
function argsOne(args:string[],name:string):string|undefined|VimCommandParse { return args.length<2?args[0]:{error:`:${name} accepts at most one count or target.`}; }
function windowCommand(key:string,count?:number):VimCommandParse {
  const focus=(direction:'left'|'right'|'up'|'down'):VimCommand=>({kind:'focus-pane',target:{kind:'direction',direction}});
  switch(key) {
    case'h':return{command:focus('left')};case'j':return{command:focus('down')};case'k':return{command:focus('up')};case'l':return{command:focus('right')};
    case't':return{command:{kind:'focus-pane',target:'first'}};case'b':return{command:{kind:'focus-pane',target:'last'}};case'w':return{command:{kind:'focus-pane',target:'next'}};case'W':return{command:{kind:'focus-pane',target:'previous'}};
    case's':return{command:{kind:'split',axis:'vertical',...(count?{size:count}:{})}};case'v':return{command:{kind:'split',axis:'horizontal',...(count?{size:count}:{})}};
    case'c':return{command:{kind:'close-pane',...(count?{target:count}:{})}};case'o':return{command:{kind:'only-pane'}};case'r':return{command:{kind:'rotate-panes',direction:1}};case'R':return{command:{kind:'rotate-panes',direction:-1}};case'x':return{command:{kind:'exchange-pane',...(count?{target:count}:{})}};
    case'H':return{command:{kind:'move-pane',edge:'left'}};case'J':return{command:{kind:'move-pane',edge:'bottom'}};case'K':return{command:{kind:'move-pane',edge:'top'}};case'L':return{command:{kind:'move-pane',edge:'right'}};
    case'=':return{command:{kind:'equalize-panes'}};case'_':return{command:{kind:'resize-pane',axis:'vertical',...(count?{size:count}:{maximize:true})}};case'|':return{command:{kind:'resize-pane',axis:'horizontal',...(count?{size:count}:{maximize:true})}};
    case'+':return{command:{kind:'resize-pane',axis:'vertical',delta:count??1}};case'-':return{command:{kind:'resize-pane',axis:'vertical',delta:-(count??1)}};case'>':return{command:{kind:'resize-pane',axis:'horizontal',delta:count??1}};case'<':return{command:{kind:'resize-pane',axis:'horizontal',delta:-(count??1)}};
    default:return{error:`Unsupported Ctrl-W command: ${key}.`};
  }
}
export function parseVimWindowKey(key:string,count?:number):VimCommandParse { if(!key||key.length!==1)return{error:'Enter one Ctrl-W command key.'};if(count!==undefined&&(!Number.isInteger(count)||count<1))return{error:'Ctrl-W count must be a positive integer.'};return windowCommand(key,count); }
export function completeVimCommand(value:string):VimCompletion[] { const q=value.trimStart().replace(/^:/,'').toLowerCase(); if(/\s+\S/.test(q))return[];return specs.filter(([name])=>name.startsWith(q)).map(([name,detail])=>({value:name,label:`:${name}`,detail})); }

/** Parse the bounded Ex vocabulary with native Monitter actions. */
export function parseVimCommand(value:string):VimCommandParse {
  const source=value.trim();if(!source)return{error:'Enter a Monitter command, for example :tabnew.'};
  const match=source.replace(/^:/,'').match(/^([+-]?\d+|[+$-])?\s*([a-zA-Z]+)(?:\s+(.*))?$/);if(!match)return{error:`Invalid Monitter command: :${source.replace(/^:/,'')}.`};
  const [,prefix='',rawName,rawArgs='']=match,name=rawName.toLowerCase(),args=rawArgs?rawArgs.trim().split(/\s+/):[];
  if(name==='vertical'||name==='vert') { if(args[0]?.toLowerCase()!=='resize'||args.length>2)return{error:':vertical only supports :vertical resize [N].'};return resize('horizontal',args[1]); }
  if(name==='wincmd') { if(args.length!==1)return{error:':wincmd needs exactly one Ctrl-W command key.'};const count=prefix?(positive(prefix)??undefined):undefined;if(prefix&&!count)return{error:':wincmd only accepts a positive numeric count.'};return parseVimWindowKey(args[0],count); }
  const one=argsOne(args,name);if(typeof one!=='string'&&one!==undefined)return one;
  const tab=one?target(one):prefix?target(prefix):undefined;
  const rawTarget=one??prefix;
  if((one||prefix)&&!tab && ['tabnew','tabedit','tabe','tabnext','tabn','tabprevious','tabp','tabclose','tabc','q','quit','tabonly','tabo'].includes(name))return{error:`Invalid tab target: ${rawTarget}.`};
  switch(name) {
    case'tabnew':case'tabedit':case'tabe':return{command:{kind:'tabnew',...(tab?{after:tab}:{})}};
    case'tabnext':case'tabn':return{command:{kind:'tabselect',target:tab??{kind:'relative',offset:1}}};
    case'tabprevious':case'tabp':return{command:{kind:'tabselect',target:tab??{kind:'relative',offset:-1}}};
    case'tabfirst':case'tabrewind':case'tabfir':if(one||prefix)return{error:`:${name} does not take arguments.`};return{command:{kind:'tabselect',target:{kind:'index',index:1}}};
    case'tablast':case'tabl':if(one||prefix)return{error:`:${name} does not take arguments.`};return{command:{kind:'tabselect',target:{kind:'last'}}};
    case'tabclose':case'tabc':case'q':case'quit':return{command:{kind:'tabclose',...(tab?{target:tab}:{})}};
    case'tabonly':case'tabo':return{command:{kind:'tabonly',...(tab?{target:tab}:{})}};
    case'tabmove':case'tabm': {const raw=one??prefix;if(!raw||raw==='$')return{command:{kind:'tabmove',target:'last'}};if(raw==='0')return{command:{kind:'tabmove',target:'first'}};const r=relative(raw);if(r!==null)return{command:{kind:'tabmove',target:{kind:'relative',offset:r}}};const n=positive(raw);return n?{command:{kind:'tabmove',target:{kind:'after',index:n}}}:{error:`Invalid :tabmove target: ${raw}.`};}
    case'split':case'sp':return split('vertical',prefix,one);case'vsplit':case'vs':return split('horizontal',prefix,one);
    case'terminal':case'term':if(one||prefix)return{error:`:${name} does not take arguments.`};return{command:{kind:'terminal'}};
    case'close':case'clo': {const n=positive(one??prefix);return(one||prefix)&&!n?{error:':close only accepts a positive pane number.'}:{command:{kind:'close-pane',...(n?{target:n}:{})}};}
    case'only':case'on':if(one||prefix)return{error:`:${name} does not take arguments.`};return{command:{kind:'only-pane'}};
    case'resize':case'res':return resize('vertical',one??prefix);
    case'help':if(one||prefix)return{error:':help is limited to Monitter command help.'};return{command:{kind:'help'}};
    default:return{error:`Unknown or unsupported Monitter command: :${name}. Type :help for available commands.`};
  }
}
function split(axis:'horizontal'|'vertical',prefix:string,arg:string|undefined):VimCommandParse { const raw=arg??prefix,n=raw?positive(raw):null;return raw&&!n?{error:':split only accepts a positive numeric size.'}:{command:{kind:'split',axis,...(n?{size:n}:{})}}; }
function resize(axis:'horizontal'|'vertical',raw:string|undefined):VimCommandParse { if(!raw)return{command:{kind:'resize-pane',axis,maximize:true}};const d=relative(raw);if(d!==null)return{command:{kind:'resize-pane',axis,delta:d}};const n=positive(raw);return n?{command:{kind:'resize-pane',axis,size:n}}:{error:':resize expects a positive size or +N/-N.'}; }
export const vimCommandHelp=specs.map(([name,detail])=>`:${name} — ${detail}`);
