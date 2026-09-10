import {chromium,expect} from '@playwright/test';
const browser=await chromium.launch({headless:true});
try {
 const page=await browser.newPage({viewport:{width:1440,height:900}});
 await page.addInitScript({path:'scripts/ui-fixture.js'});await page.goto('http://127.0.0.1:18433');
 await page.getByRole('button',{name:'Edit Atlas',exact:true}).click();
 const dialog=page.getByRole('dialog');const yolo=dialog.getByRole('switch',{name:/YOLO/});
 await expect(yolo).not.toBeChecked();await yolo.check();await expect(dialog.getByLabel('Permissions',{exact:true})).toHaveValue('yolo');
 await dialog.getByRole('button',{name:'Save agent',exact:true}).click();
 expect(await page.evaluate(()=>window.__MONITTER_QA__.snapshot().agents[0].sandbox)).toBe('yolo');
 await page.getByRole('button',{name:'Edit Atlas',exact:true}).click();
 await dialog.getByLabel('Harness',{exact:true}).selectOption('claude');await expect(yolo).toBeEnabled();await expect(yolo).not.toBeChecked();await yolo.check();
 for(const harness of ['opencode','hermes']){await dialog.getByLabel('Harness',{exact:true}).selectOption(harness);await expect(yolo).toBeDisabled();await expect(yolo).not.toBeChecked();await expect(dialog.locator('option[value="yolo"]')).toHaveCount(0);}
 console.log('YOLO defaults off, saves Codex selection, supports Claude, resets on harness change and is unavailable for unsupported providers.');
}finally{await browser.close();}
