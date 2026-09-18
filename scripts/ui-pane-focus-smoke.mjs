import {chromium,expect} from '@playwright/test';
const browser=await chromium.launch({headless:true});
try{
const page=await browser.newPage({viewport:{width:1500,height:950}});await page.addInitScript({path:'scripts/ui-fixture.js'});await page.addInitScript(()=>{const q=window.__MONITTER_QA__,s=q.snapshot();s.settings.shortcutMode='vim';q.setSnapshot(s);});await page.goto('http://127.0.0.1:18433');await page.getByRole('button',{name:'Preferences',exact:true}).waitFor({timeout:60000});
await page.keyboard.press('Escape');await page.keyboard.type(':');const input=page.getByRole('textbox',{name:'Monitter command'});await input.fill('vsplit');await input.press('Enter');await expect(page.locator('.pane-leaf')).toHaveCount(2);
await page.keyboard.press('Meta+w');await page.keyboard.press('ArrowLeft');await page.waitForTimeout(450);await expect(page.locator('.pane-leaf')).toHaveCount(2);
await expect(page.locator('.pane-leaf.active')).toHaveAttribute('data-pane-id','main');
await page.keyboard.press('Meta+w');await page.keyboard.press('ArrowLeft');
await expect(page.locator('.sidebar')).toHaveClass(/sidebar-selected/);
await expect(page.locator('.pane-leaf.active')).toHaveCount(0);
await expect(page.getByRole('tab',{name:'Agents view'})).toBeFocused();
await page.keyboard.press('ArrowRight');await expect(page.getByRole('tab',{name:'Projects view'})).toHaveAttribute('aria-selected','true');
await page.keyboard.press('ArrowDown');await expect(page.getByRole('tab',{name:'Activity view'})).toBeFocused();
await page.getByRole('separator',{name:'Resize main sidebar'}).focus();await page.keyboard.press('Enter');
await expect(page.locator('.project-rail')).toBeVisible();
await expect(page.locator('.sidebar-rail-view')).toBeFocused();
await page.keyboard.press('ArrowRight');await expect(page.locator('.activity-rail')).toBeVisible();
await page.keyboard.press('Meta+w');await page.keyboard.press('ArrowRight');await expect(page.locator('.pane-leaf.active')).toHaveAttribute('data-pane-id','main');
await page.keyboard.press('Meta+w');await page.keyboard.press('ArrowRight');await page.waitForTimeout(450);await expect(page.locator('.pane-leaf')).toHaveCount(2);
await expect(page.locator('.pane-leaf.active')).not.toHaveAttribute('data-pane-id','main');
await page.keyboard.press('Escape');await page.keyboard.type(':');await page.getByRole('textbox',{name:'Monitter command'}).fill('q');await page.keyboard.press('Enter');await expect(page.locator('.pane-leaf')).toHaveCount(1);await expect(page.locator('.pane-leaf[data-pane-id="main"]')).toBeVisible();
console.log('W-arrow navigation includes keyboard-operable expanded and collapsed sidebar; :q closes the selected secondary pane.');
}finally{await browser.close();}
