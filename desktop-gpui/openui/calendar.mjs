// Date values cross the OpenUI bridge as ISO strings, including range endpoints.
export function dateKey(value) {
 if(typeof value!=='string')return '';
 const key=value.slice(0,10);
 if(!/^\d{4}-\d{2}-\d{2}$/.test(key))return '';
 const parsed=new Date(key+'T00:00:00.000Z');
 return Number.isFinite(parsed.getTime())&&parsed.toISOString().slice(0,10)===key?key:'';
}
export function monthKey(value) {
 return dateKey(value)?.slice(0,7)||new Date().toISOString().slice(0,7);
}
export function monthDays(month,delta=0) {
 const match=/^(\d{4})-(\d{2})$/.exec(month);
 const base=match?new Date(`${month}-01T00:00:00.000Z`):new Date();
 if(!Number.isFinite(base.getTime()))return monthDays(new Date().toISOString().slice(0,7),delta);
 const first=new Date(base);first.setUTCDate(1);first.setUTCMonth(first.getUTCMonth()+delta);
 const key=first.toISOString().slice(0,7);
 const last=new Date(first);last.setUTCMonth(last.getUTCMonth()+1);last.setUTCDate(0);const count=last.getUTCDate();
 return {key,label:['January','February','March','April','May','June','July','August','September','October','November','December'][first.getUTCMonth()]+' '+first.getUTCFullYear(),offset:first.getUTCDay(),days:Array.from({length:count},(_,i)=>`${key}-${String(i+1).padStart(2,'0')}`)};
}
export function chooseDate(current,key,range=false) {
 const day=dateKey(key);if(!day)return current;
 const iso=day+'T00:00:00.000Z';
 if(!range)return iso;
 const from=dateKey(current?.from);
 if(!from||dateKey(current?.to))return {from:iso};
 return day<from?{from:iso,to:from+'T00:00:00.000Z'}:{from:from+'T00:00:00.000Z',to:iso};
}
