export type PaneLeaf={id:string}; export type PaneSplit={id:string;axis:'horizontal'|'vertical';ratio:number;first:PaneLayout;second:PaneLayout}; export type PaneLayout=PaneLeaf|PaneSplit;
export type PaneTabTransfer={sourcePaneId:string;kind:'task'|'draft'|'channel'|'terminal'|'settings';id:string};
export const leaf=(id:string):PaneLeaf=>({id});
export const split=(id:string,axis:PaneSplit['axis'],first:PaneLayout,second:PaneLayout,ratio=.5):PaneSplit=>({id,axis,ratio,first,second});
export const paneIds=(layout:PaneLayout):string[]=>'axis'in layout?[...paneIds(layout.first),...paneIds(layout.second)]:[layout.id];
