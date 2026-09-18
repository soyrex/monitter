export { MAX_WORKSPACE_PANES } from './pane-constants.js';
export type PaneLeaf={id:string}; export type PaneSplit={id:string;axis:'horizontal'|'vertical';ratio:number;first:PaneLayout;second:PaneLayout}; export type PaneLayout=PaneLeaf|PaneSplit;
export type PaneTabTransfer={sourcePaneId:string;kind:'task'|'draft'|'channel'|'terminal'|'settings'|'empty';id:string};
export type PaneLayoutPreset='single'|'columns'|'grid'|'columns-3'|'columns-4'|'grid-3x2'|'grid-4x2';
const presetDimensions:Record<PaneLayoutPreset,readonly [columns:number,rows:number]>={
  single:[1,1],columns:[2,1],grid:[2,2],
  'columns-3':[3,1],'columns-4':[4,1],
  'grid-3x2':[3,2],'grid-4x2':[4,2],
};
export const panePresetDimensions=(mode:PaneLayoutPreset):readonly [number,number]=>presetDimensions[mode];
export function createPanePresetLayout(mode:PaneLayoutPreset,ids:string[],splitId:()=>string):PaneLayout {
  const [columns,rows]=panePresetDimensions(mode);
  if(ids.length!==columns*rows)throw new Error('Pane count does not match layout preset');
  const columnLayout=(rowIds:string[]):PaneLayout=>{
    if(rowIds.length===1)return {id:rowIds[0]};
    const midpoint=Math.ceil(rowIds.length/2);
    return {id:splitId(),axis:'horizontal',ratio:midpoint/rowIds.length,
      first:columnLayout(rowIds.slice(0,midpoint)),second:columnLayout(rowIds.slice(midpoint))};
  };
  return rows===1?columnLayout(ids)
    :{id:splitId(),axis:'vertical',ratio:.5,
      first:columnLayout(ids.slice(0,columns)),second:columnLayout(ids.slice(columns))};
}
export const leaf=(id:string):PaneLeaf=>({id});
export const split=(id:string,axis:PaneSplit['axis'],first:PaneLayout,second:PaneLayout,ratio=.5):PaneSplit=>({id,axis,ratio,first,second});
export const paneIds=(layout:PaneLayout):string[]=>'axis'in layout?[...paneIds(layout.first),...paneIds(layout.second)]:[layout.id];

const paneLeafCount=(layout:PaneLayout):number=>'axis'in layout
  ? paneLeafCount(layout.first)+paneLeafCount(layout.second)
  : 1;

/** Give every leaf pane the same share of the workspace while preserving its topology. */
export function balancePaneLayout(layout:PaneLayout):PaneLayout {
  if(!('axis'in layout))return layout;
  const first=balancePaneLayout(layout.first),second=balancePaneLayout(layout.second);
  const firstCount=paneLeafCount(first),secondCount=paneLeafCount(second);
  return {...layout,ratio:firstCount/(firstCount+secondCount),first,second};
}
