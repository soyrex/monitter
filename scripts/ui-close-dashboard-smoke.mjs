import {chromium,expect} from '@playwright/test';
const browser=await chromium.launch({headless:true});
try {
 const page=await browser.newPage({viewport:{width:1440,height:900}}),errors=[];page.on('pageerror',e=>errors.push(e.message));await page.addInitScript({path:'scripts/ui-fixture.js'});await page.goto('http://127.0.0.1:18433',{timeout:60000});
 const panes=page.locator('.pane-leaf');
 async function columns(){await expect(page.getByRole('button',{name:'Standard view',exact:true})).toBeEnabled();await page.keyboard.press('Meta+p');await page.getByRole('button',{name:'Two columns',exact:true}).click();await expect(panes).toHaveCount(2);}
 await columns();await panes.last().getByRole('button',{name:'Close empty pane',exact:true}).click();await expect(panes).toHaveCount(1);
 await columns();await panes.last().click({position:{x:100,y:150}});await page.keyboard.press('Meta+,');await expect(panes.last().locator('.settings-pane')).toBeVisible();
 await expect(panes).toHaveCount(2);await expect(panes.last().getByRole('button',{name:'Overview',exact:true})).toHaveCount(0);
 await panes.first().getByRole('button',{name:'Close dashboard tab',exact:true}).click();await expect(panes).toHaveCount(1);await expect(panes.first().locator('.settings-pane')).toBeVisible();await expect(panes.first()).toHaveAttribute('data-pane-id','main');
 await page.reload();await expect(panes).toHaveCount(1);await expect(panes.first().locator('.settings-pane')).toBeVisible();await expect(panes.first().getByRole('button',{name:'Overview',exact:true})).toHaveCount(0);
 await columns();await panes.last().getByRole('region',{name:'Choose pane content'}).click();await page.keyboard.press('Meta+w');await expect(panes).toHaveCount(1);await expect(panes.first().locator('.settings-pane')).toBeVisible();await panes.first().getByRole('button',{name:'Close Settings tab',exact:true}).click();
 await columns();await panes.last().getByRole('region',{name:'Choose pane content'}).click();await page.getByRole('button',{name:'New chat with Atlas',exact:true}).click();
 await panes.last().getByRole('textbox',{name:'Task message',exact:true}).fill('Keep this unsent draft');
 await panes.first().getByRole('button',{name:'Close empty pane',exact:true}).click();await expect(panes).toHaveCount(1);await expect(panes.first().getByRole('textbox',{name:'Task message',exact:true})).toHaveValue('Keep this unsent draft');
 await page.reload();await expect(panes.first().getByRole('textbox',{name:'Task message',exact:true})).toHaveValue('Keep this unsent draft');expect(errors).toEqual([]);
 console.log('Empty pane close, occupied dashboard close, main pane promotion, saved layout and Cmd-W passed.');
}finally{await browser.close();}
