import {chromium,expect} from '@playwright/test';
import {existsSync,writeFileSync} from 'node:fs';
const chrome='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const browser=await chromium.launch({headless:true,...(existsSync(chrome)?{executablePath:chrome}:{})});
const page=await browser.newPage({viewport:{width:1440,height:1000}});
try {
 await page.addInitScript({path:'scripts/ui-fixture.js'});
 await page.goto(process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18433');
 await expect(page.getByRole('button',{name:'Preferences',exact:true})).toBeVisible({timeout:60000});
 await page.getByRole('button',{name:'Preferences',exact:true}).click();
 await page.getByRole('navigation',{name:'Settings categories'}).getByRole('button',{name:'Typography',exact:true}).click();
 await expect.poll(()=>page.locator('#text-font-choices option[value="Fixture Sans"]').count()).toBe(1);
 await expect.poll(()=>page.locator('#terminal-font-choices option[value="Fixture Mono"]').count()).toBe(1);
 for (const [label,value] of [['Interface font','Avenir Next'],['Chat font','Georgia'],['Terminal font','Menlo'],['Interface font base size','18'],['Chat font base size','20'],['Terminal font base size','16']]) {
  const input=page.getByLabel(label,{exact:true});
  await input.fill(value);await input.press('Tab');
  await expect(input).toBeEnabled();
 }
 await expect.poll(()=>page.evaluate(()=>getComputedStyle(document.documentElement).getPropertyValue('--chat-font-size').trim())).toBe('20px');
 await expect.poll(()=>page.evaluate(()=>window.__MONITTER_QA__.snapshot().settings.terminalFontSize)).toBe(16);
 const values=await page.evaluate(()=>{
  const css=getComputedStyle(document.documentElement), settings=window.__MONITTER_QA__.snapshot().settings;
  return {settings,interfaceFont:css.fontFamily,chatFont:css.getPropertyValue('--chat-font'),terminalFont:css.getPropertyValue('--terminal-font'),terminalSize:css.getPropertyValue('--terminal-font-size'),interfaceRatio:css.getPropertyValue('--interface-font-ratio')};
 });
 expect(values.settings.interfaceFontSize).toBe(18);expect(values.settings.chatFontSize).toBe(20);expect(values.settings.terminalFontSize).toBe(16);
 expect(values.interfaceFont).toContain('Avenir Next');expect(values.chatFont).toContain('Georgia');expect(values.terminalFont).toContain('Menlo');expect(Number(values.terminalSize)).toBe(16);expect(Number(values.interfaceRatio)).toBeCloseTo(18/14);
 await page.getByLabel('Chat font',{exact:true}).fill('');await page.getByLabel('Chat font',{exact:true}).press('Tab');
 await expect.poll(()=>page.evaluate(()=>getComputedStyle(document.documentElement).getPropertyValue('--chat-font'))).toContain('IBM Plex Sans');
 writeFileSync('verification/ui-font-settings.json',JSON.stringify({passed:['local system font suggestions populate font selectors','independent font faces and sizes saved and applied','interface ratio preserves hierarchy','clearing font restores default']},null,2));
 console.log('Font settings UI checks passed');
} finally {await browser.close();}
