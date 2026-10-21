import test from 'node:test';
import assert from 'node:assert/strict';
import {dateKey,monthDays,chooseDate} from './calendar.mjs';
test('calendar validates dates and handles leap years and month boundaries',()=>{
 assert.equal(dateKey('2024-02-29T23:59:00Z'),'2024-02-29');
 assert.equal(dateKey('2025-02-29'),'');
 assert.equal(monthDays('2024-02').days.length,29);
 assert.equal(monthDays('2026-12',1).key,'2027-01');
 assert.equal(monthDays('2026-01',-1).key,'2025-12');
});
test('single and range selections emit serializable ISO values in date order',()=>{
 assert.equal(chooseDate(null,'2026-10-21'),'2026-10-21T00:00:00.000Z');
 const first=chooseDate(null,'2026-10-21',true);
 assert.deepEqual(first,{from:'2026-10-21T00:00:00.000Z'});
 assert.deepEqual(chooseDate(first,'2026-10-20',true),{from:'2026-10-20T00:00:00.000Z',to:'2026-10-21T00:00:00.000Z'});
 assert.deepEqual(chooseDate({from:'2026-10-20',to:'2026-10-21'},'2026-10-22',true),{from:'2026-10-22T00:00:00.000Z'});
});
