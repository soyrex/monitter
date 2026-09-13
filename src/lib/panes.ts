export type PaneLeaf={id:string}; export type PaneSplit={id:string;axis:'horizontal'|'vertical';ratio:number;first:PaneLayout;second:PaneLayout}; export type PaneLayout=PaneLeaf|PaneSplit;
export type PaneTabTransfer={sourcePaneId:string;kind:'task'|'draft'|'channel'|'terminal'|'settings'|'empty';id:string};
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
