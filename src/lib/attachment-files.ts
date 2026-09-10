import type {AttachmentFileData} from './types';
export const attachmentLimit=20*1024*1024;
export async function readBrowserFile(file:File):Promise<AttachmentFileData>{
  if(file.size>attachmentLimit)throw new Error(`${file.name} is larger than the 20 MB attachment limit.`);
  const dataBase64=await new Promise<string>((resolve,reject)=>{const reader=new FileReader();reader.onload=()=>resolve(String(reader.result).split(',')[1]);reader.onerror=()=>reject(new Error(`Could not read ${file.name}.`));reader.readAsDataURL(file)});
  return {filename:file.name||'pasted-file',mimeType:file.type||'application/octet-stream',dataBase64};
}
export async function thumbnail(file:Blob):Promise<string|null>{
  if(!file.type.startsWith('image/'))return null;
  let bitmap:ImageBitmap|undefined;
  try{
    bitmap=await createImageBitmap(file);
    const scale=Math.min(1,192/Math.max(bitmap.width,bitmap.height)),canvas=document.createElement('canvas');
    canvas.width=Math.max(1,Math.round(bitmap.width*scale));canvas.height=Math.max(1,Math.round(bitmap.height*scale));
    canvas.getContext('2d')?.drawImage(bitmap,0,0,canvas.width,canvas.height);
    const result=canvas.toDataURL('image/png');return result.length<=256*1024?result:null;
  }catch{return null}finally{bitmap?.close()}
}
export function nativeBlob(file:AttachmentFileData):Blob{
  const decoded=atob(file.dataBase64),bytes=new Uint8Array(decoded.length);for(let index=0;index<decoded.length;index++)bytes[index]=decoded.charCodeAt(index);
  return new Blob([bytes],{type:file.mimeType});
}
