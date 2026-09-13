function rgb(hex:string):[number,number,number] {
  const value=hex.trim().replace(/^#/,'');
  const normalized=value.length===3 ? [...value].map(channel=>channel+channel).join('') : value;
  if(!/^[0-9a-f]{6}$/i.test(normalized))return [63,157,106];
  return [0,2,4].map(index=>Number.parseInt(normalized.slice(index,index+2),16)) as [number,number,number];
}

function relativeLuminance(hex:string) {
  return rgb(hex).map(channel=>{
    const value=channel/255;
    return value<=.04045 ? value/12.92 : ((value+.055)/1.055)**2.4;
  }).reduce((sum,value,index)=>sum+value*[.2126,.7152,.0722][index],0);
}

/** Choose the higher-contrast monochrome foreground for a solid colour. */
export function contrastForeground(background:string):'#000'|'#fff' {
  const luminance=relativeLuminance(background);
  const blackContrast=(luminance+.05)/.05;
  const whiteContrast=1.05/(luminance+.05);
  return blackContrast>=whiteContrast ? '#000' : '#fff';
}
