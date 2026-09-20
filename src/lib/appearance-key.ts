import type { AppThemeSelection } from './app-theme';
import type { Settings } from './types';

/** Only inputs consumed by AppSurface's document-wide appearance renderer.
 * Snapshots are immutable and arrive with fresh settings objects while agents
 * stream, so reference equality cannot suppress unchanged appearance work.
 */
export function appearanceKey(settings: Settings, theme: AppThemeSelection, scale: number, tint: number, nativeRuntime: boolean): string {
  return JSON.stringify([
    settings.theme, settings.windowSurface, settings.interfaceDensity,
    settings.interfaceFontSize, settings.chatFontSize, settings.terminalFontSize,
    settings.chatLineHeight, settings.terminalLineHeight, settings.windowTransparency,
    settings.interfaceFont, settings.chatFont, settings.terminalFont,
    theme.light, theme.dark, theme.accent, theme.contrast, scale, tint, nativeRuntime,
  ]);
}

/** System colour-scheme changes affect browser chrome only in system mode. */
export function browserChromeKey(settings: Settings, theme: AppThemeSelection, tint: number, systemDark: boolean): string {
  return JSON.stringify([
    settings.theme, settings.windowSurface, settings.theme === 'system' && systemDark,
    theme.light, theme.dark, theme.accent, theme.contrast, tint,
  ]);
}
