import { chromium, expect } from '@playwright/test';
const browser=await chromium.launch({headless:true});
try {
  const page=await browser.newPage({viewport:{width:1440,height:1000}});
  await page.addInitScript({path:'scripts/ui-fixture.js'});
  await page.goto(process.env.TEST_URL ?? 'http://127.0.0.1:18450/',{timeout:180000});
  await page.locator('.sidebar').waitFor({timeout:180000});
  for(const theme of ['light','dark']) {
    await page.evaluate(theme=>{const qa=window.__MONITTER_QA__,s=qa.snapshot();s.settings.theme=theme;s.settings.accent='#29b6f6';qa.setSnapshot(s);},theme);
    await expect(page.locator('html')).toHaveAttribute('data-theme',theme);
    await expect.poll(()=>page.evaluate(()=>document.documentElement.style.getPropertyValue('--accent'))).toBe('#29b6f6');
    const colors=await page.evaluate(()=>{
      const root=getComputedStyle(document.documentElement);
      const result={};
      for(const name of ['paper','sidebar','panel','soft','code','line']) result[name]=root.getPropertyValue(`--${name}`).trim();
      result.ink=root.getPropertyValue('--ink').trim();
      result.factor=root.getPropertyValue('--surface-tint-factor').trim();
      return result;
    });
    for(const name of ['paper','sidebar','panel','soft','code','line']) expect(colors[name]).toContain('5%');
    expect(colors.ink).toBe(theme==='dark'?'#eeeeee':'#252525');
    expect(Number(colors.factor)).toBe(theme==='dark'?1:0.5);
    await expect(page.locator('.workspace-health')).toContainText('1 agent · 0 running');
    await page.screenshot({path:`/tmp/monitter-neutral-${theme}.png`});
    console.log(theme,colors);
  }
  await page.getByRole('button',{name:'Preferences',exact:true}).click();
  await page.locator('.settings-nav button').filter({hasText:'Appearance'}).click();
  const slider=page.getByRole('slider',{name:'Tint intensity',exact:true});
  for(const value of [0,20,50]) {
    await slider.evaluate((input,value)=>{input.value=String(value);input.dispatchEvent(new Event('input',{bubbles:true}));},value);
    await expect.poll(()=>page.evaluate(()=>document.documentElement.style.getPropertyValue('--surface-tint'))).toBe(`${value}%`);
  }
  await page.reload();
  await expect(slider).toHaveValue('50');
  await page.getByRole('button',{name:'Reset to default · 5%',exact:true}).click();
  await expect(slider).toHaveValue('5');
  console.log('Tint slider previews 0–50%, survives reload and resets to 5%.');
} finally {await browser.close();}
