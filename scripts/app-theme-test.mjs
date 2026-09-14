import assert from 'node:assert/strict';
import { appThemePreset, appThemes, DEFAULT_APP_THEME, mixThemeColour, normalizeAppTheme, normalizeAppThemeSelection } from '../src/lib/app-theme.ts';

assert.equal(appThemes.length,10);
assert.deepEqual(appThemes.map(theme=>theme.label),[
  'Monitter','Catppuccin','Dracula','Gruvbox','Nord',
  'One','Solarized','Tokyo Night','Wombat','XTerm',
]);
assert.equal(normalizeAppTheme('nord'),'nord');
assert.equal(normalizeAppTheme('unknown'),'monitter');
assert.equal(appThemePreset('monitter').light.accent,'#00A8F0');
assert.equal(appThemePreset('monitter').dark.accent,'#00A8F0');
assert.equal(mixThemeColour('#F0F0F0','#00A8F0',0.025),'#EAEEF0');
assert.equal(mixThemeColour('#0F0F0F','#00A8F0',0.05),'#0E171A');
assert.deepEqual(normalizeAppThemeSelection({light:'catppuccin',dark:'nord',accent:'#123ABC'}),{light:'catppuccin',dark:'nord',accent:'#123ABC'});
assert.deepEqual(normalizeAppThemeSelection({light:'unknown',dark:null,accent:'red'}),DEFAULT_APP_THEME);
for (const theme of appThemes) {
  assert.equal(appThemePreset(theme.id),theme);
  for (const palette of [theme.light,theme.dark]) {
    for (const value of Object.values(palette)) assert.match(value,/^#[0-9a-f]{6}$/i);
  }
}
console.log('ten paired whole-app palettes and selection normalization are valid');
