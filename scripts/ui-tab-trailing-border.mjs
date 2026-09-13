import { readFileSync } from 'node:fs';
import { chromium, webkit, expect } from '@playwright/test';

// Exercise the actual tab CSS at fractional interface scales, without app data.
const source = readFileSync('src/lib/components/AppSurface.svelte', 'utf8');
const css = source.match(/<style>([\s\S]*?)<\/style>/)[1];
for (const engine of [chromium, webkit]) {
  const browser = await engine.launch();
  try {
    const page = await browser.newPage({ viewport: { width: 1800, height: 900 } });
    for (const modern of [false, true]) {
      await page.setContent(`<style>${css}</style><style>
        body { margin:0; --line:rgb(120,120,120); --paper:#181e25; }
        .workspace { display:block; width:601px; }
        .tabs { height:40px; }
        .tab-entry { width:137.3px; box-sizing:border-box; }
      </style><main class="workspace ${modern ? 'modern-tabs' : ''}"><div class="tabs"><div class="tab-picker-list">${Array.from({length:8}, (_,i)=>`<div class="tab-entry ${i===7?'active':''}">${i}</div>`).join('')}</div></div></main>`);
      for (const scale of [1, 1.25, 1.65, 2]) {
        const bounds = await page.evaluate(scale => {
          document.querySelector('.workspace').style.zoom = scale;
          const strip = document.querySelector('.tabs');
          strip.scrollLeft = strip.scrollWidth;
          const last = strip.querySelector('.tab-entry:last-child');
          return {
            clearance: (strip.getBoundingClientRect().right - last.getBoundingClientRect().right) / scale,
            border: getComputedStyle(last).borderRightWidth,
            color: getComputedStyle(last).borderRightColor,
          };
        }, scale);
        expect(parseFloat(bounds.border) * scale).toBeGreaterThanOrEqual(1);
        expect(bounds.color).toBe('rgb(120, 120, 120)');
        expect(bounds.clearance).toBeGreaterThanOrEqual(1);
        expect(bounds.clearance).toBeLessThan(3);
      }
    }
    console.log(`${engine.name()}: trailing border retained in both tab styles at 100–200% scale`);
  } finally { await browser.close(); }
}
