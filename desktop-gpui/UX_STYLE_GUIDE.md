# Monitter interface style guide

Version 1 · 7 October 2026 · GPUI desktop experiment

This is the working reference for the native desktop. The Tauri interface supplies
the visual identity, chat hierarchy, typography, and control placement. Shared
tokens provide a more predictable rhythm as the native interface evolves.

## Character

Monitter is a compact workspace for talking to agents. Conversations take priority;
navigation and execution details stay quiet until useful. Use readable text, a
small number of surface colors, fine dividers, and restrained motion. Prefer
retained native controls and familiar keyboard behavior.

- Preserve the Modern tabs, curated activity tree, pane-local secondary sidebar,
  Git strip above the composer, and Access/Jev/Model controls below it.
- Keep persistent running status separate from hover and keyboard focus.
- Make related controls look related through proximity and alignment.
- Use whitespace to establish groups before adding another border or card.
- Keep configurable themes, accents, fonts, density, and scale.

## Spacing

The source of truth is [`src/design.rs`](src/design.rs): `design::space`.
Values are **design pixels**, converted once through `design::ui_scaled_px`.
At 125% interface scale, 10 design pixels occupy 12.5 native points. Retina image
pixels are a separate measurement. GPUI's rem-based `.gap_2()` is not this scale.

| Token | Value | Role |
| --- | ---: | --- |
| `TIGHT` | 5 | Icon/label and related-control gaps; compact internal groups |
| `INSET` | 10 | Standard card/row padding; transcript item separation |
| `GROUP` | 15 | Established Tauri pane-header inset; intermediate breathing room |
| `SECTION` | 20 | Distinct sections; transcript side gutters |
| `PAGE` | 30 | Preferences page inset and large content boundaries |

Use a token for a role, rather than rounding every dimension to a multiple of five.
Borders, glyph alignment, optical icon sizes, native window controls, and resize
targets have different jobs. Document exceptions beside their implementation.
Apply spacing to the container that owns it: avoid doubled child/parent padding.
Virtualized transcript rows own their trailing separation as padding in their
measured outer wrapper. A root margin is not included reliably in the list's
cached row height and can disappear between adjacent entries.
Scrolling sidebar lists preserve each row's natural height; rows, section labels,
and provider tiles must not shrink to fit the viewport. Overflow scrolls instead
of clipping the second line of metadata.

### Alignment recipes

| Surface | Insets and separation |
| --- | --- |
| Chat transcript | 20 side gutters, 10 top/bottom; equal 10 gaps for bubbles and activity trees |
| Message card | 10 padding; 5 between identity metadata and text |
| Composer | 10 internal padding; 5 between input controls; normal outer inset 10; 5 corner radius, context meter inside curved outline |
| Pane header | 15 horizontal inset; 10 between identity and actions |
| Sidebar sections | 10 list inset; 5 within related rows; 20 between sections; provider tile, two text lines, quiet trailing time/status |
| Sidebar header | Center the optical wordmark in both axes; view-tab bottom aligns with the adjacent dock strip plus pane header |
| Subagent companion | Active/Recent identity rows in the owning pane; selected child tabs and bounded transcript visor below the composer |
| Usage footer | Full-width dividers; rings 10 top/bottom; resources 10 on all sides |
| Preferences | 30 page inset; 20 between sections; cards 10 inside; related controls 5 apart |
| Popup menu | 10 body inset; 5 between related controls; descriptions wrap |

Tight density reduces the composer inset/header vertical padding to 5; normal
uses 10; spacious uses a 20 composer inset and 15 header vertical padding.
Typography and native window controls keep their own sizing rules.

Standard vendor menus retain their component-owned 8px side inset and compact
row geometry. Rich Access/Model menu content uses the shared 5px internal rhythm;
the component retains its own outer padding and radius. Do not stack another
10px wrapper onto those menus just to satisfy the table.

## Typography and icons

New interface surfaces must use the validated interface font family and size from the active theme (`cx.theme().font_family` and `font_size`), including their buttons. Do not inherit the transcript font or introduce independent font sizes. Sidebar approval reviews are single-line, truncated navigation rows; request details belong in the owning pane modal. Approval popups start brief: summary, decisions and Show details; expand scope, commands and technical payloads on demand.

Use the bundled **IBM Plex Sans** for interface/chat text and **IBM Plex Mono**
for compact technical controls, paths, timings, and resources. Defaults remain
14 for interface text and 13 for chat, with the user's font settings respected.
Use semibold for section names, normal weight for body text, and muted color for
supporting metadata. Pane titles retain the Tauri 13.2 bold optical size.

Keep small technical labels around 10–11 design pixels and avoid reducing them
to solve layout overflow. Truncate tab titles, wrap menu descriptions, and give
numeric columns a stable width. Tool previews should use meaningful English.

Use existing bundled icon artwork. The tab's icon identifies its content type;
provider artwork identifies the provider. The sidebar wordmark uses prefiltered transparent PNG variants chosen for the actual device width, retaining its optical size. Avoid replacing known marks with
letters when real artwork exists. Icons remain legible at the current theme's
contrast. Decorative backgrounds do not replace text labels or accessible names.

## Surfaces, borders, and corners

| Token | Value | Role |
| --- | ---: | --- |
| `radius::CONTROL` | 5 | Composer, small menu/control containers |
| `radius::CARD` | 10 | Message cards, form cards |

Keep Tauri's directional 3px message corner as an intentional exception. Native
Modern tab curves and OS window corners retain their established geometry.
Use one fine border to express a boundary; avoid nested boxes around ordinary
sidebar rows. Usage and resource panels extend to the sidebar edges.

Apply the same configured alpha to ordinary background surfaces. Foreground
text/icons retain full contrast. A container that only lays out child panes
should not introduce another background alpha layer. Color differences separate
paper/sidebar/panel/soft surfaces. Bubble outlines use the same alpha as their
background surfaces. Modals and menus use one quarter of the configured
transparency, over a strongly blurred capture of the actual underlying scene.
Dark menus deepen the panel tint to preserve copy contrast.
Transparent menus also blur underlying in-app text; capture once before opening,
share it across nested popups, and release it after the last popup closes. Native
window vibrancy blurs the desktop behind translucent panes.

## Buttons and menus

Use existing compact native buttons for dense toolbars. Current composer controls
retain their 28px height; general compact actions commonly use 30px. These are
hitbox/optical dimensions, separate from the spacing scale. Icon-only actions
need an accessible name and a tooltip. Primary Send retains its clear outlined
treatment; show Stop when a turn is running.

| State | Treatment |
| --- | --- |
| Idle secondary action | Quiet foreground, transparent background |
| Hover | Soft background, readable foreground; sidebar transition 180ms |
| Keyboard focus | Visible focus outline without relying on hover |
| Selected/active | Persistent soft/accent treatment, independent of pointer |
| Disabled | Subdued appearance and disabled action; explanation where useful |
| Busy | Truthful state and bounded indicator; preserve labels and hitbox |
| Error | Scoped message, clear recovery/dismissal where supported |

Access and Model open menus above the clicked button. Access aligns to the left
edge; Model aligns to the right. Constrain them to the viewport, wrap explanatory
notes, and scroll long catalogs. Large catalogs provide a search field. Model rows
show the real description and selected checkmark; reasoning uses an ordered
capability-driven slider, and Fast appears only when supported. Ordinary model selection stays in this menu;
full preferences stay in the preferences modal. Pane ellipsis contains that
pane's actions. Group destructive actions separately and label them precisely.
The chat pane menu offers Rename chat and Archive chat. Rename also accepts
`/rename <title>`; `/rename` opens the title editor. Archive is recoverable and
removes only that chat's open pane, preserving its history. Running chats cannot
be archived.

## Tabs and dividers

- Retain Modern tab geometry, compact height, content-type icon, and truncated
  title. Do not add card-like spacing between the active tab and pane header.
- Active tab and pane header read as a continuous surface. Keep the top divider
  across the rest of the dock; interrupt it where the active tab joins.
- Empty tab-strip space drags the window when no tab drag is underway. During a
  tab drag, that entire region remains a tab drop target.
- Close controls appear on hover/focus and remain keyboard accessible. Closing a
  GUI pane never implies stopping the underlying agent run.
- Divider hover lights the full divider in Monitter blue. Maintain a forgiving
  invisible hit area; do not add a pill handle. Release, missed release, and
  window deactivation must cancel resizing reliably.
- The compact sidebar follows the native traffic-light cluster width. Expanded
  sidebar resizing preserves the user's selected width, capped at 360 design
  pixels. Saved-width restoration must never interrupt a held divider gesture.
  Usage rings share one row and grow to at most 64 design pixels; further width
  enlarges the gaps. The clock remains four pixels larger than the rings.

## Activity and chat hierarchy

User text, agent text, and curated execution steps share the same outer spacing.
A collapsed turn shows Thinking/Pondering/Working plus its tool count and the
latest useful English tool preview. Expansion reveals the actual step tree.
Usage/accounting events and saved approval cards stay out of this conversation
hierarchy; pending requests remain visible.

Hover over the owning message reveals Copy, Reply, and Fork without a permanent history toolbar.
Messages, activity steps, approvals, and composer controls share one centred
column, capped at 700 design pixels and shrinking to fit a narrow pane. The pane
header and viewport remain full bleed. Empty single-line composers use consistent
32px icon controls and the same top, right, and bottom inset around Send.
Loading older history controls appear only at the reached history boundary.
A 24px alpha fade at both transcript viewport edges fades content as it scrolls
under the header and composer, preserving the transparent background.
Keep date separators uppercase. The context meter is painted inside the composer's
rounded inner outline, including at full usage. Git metadata occupies its own strip
above the composer only when present.

Child-agent navigation separates Active and Recent identity rows in the pane-local
sidebar. A selected child opens a bounded transcript visor below the composer;
closing the visor changes only presentation. Its body uses a distinct code surface
and selectable monospaced, literal child output; it does not reinterpret reports as
chat bubbles or substitute the parent prompt for missing child output. Completed-only history starts closed.
Keep the visor close button outside the horizontally scrolling child tabs, fixed
with a right inset and reachable even in a narrow pane. The visor spans the pane's
full width and reaches its bottom edge, with padding inside the panel. Opening and
closing slide over 250ms; the upper chat layout and its sparkle field shrink and
expand together. Closing retains the transcript until the slide finishes. Reduced
motion switches immediately without starting a frame loop.

Running sparkles fade in/out over 500ms. Active tab and pane header share their
coordinates and phase where adjacent. Respect reduced motion, retain text status,
and avoid invalidating the whole desktop for a small animated child.

## Preferences and dialogs

Preferences occupies approximately 90% of the window, with a blurred blockout,
clear category navigation, a reachable close action, and independent scrolling.
Standard section/card padding follows the table above. Modal focus stays inside
the modal; Escape closes it, and background shortcuts remain blocked.

Supporting dialogs use the same surface/corner/spacing rules while fitting their
content. A popup menu should remain a popup menu. Provider/access authority,
settings values, and agent execution remain owned by the existing backend.

## Settings form controls

Use the existing GPUI Kit control families with the composed application theme.
Keep native focus, text selection, keyboard navigation, and disabled handling.
The common form layout is a 10px-inset section card, 5px label/control/help gaps,
and 20px between distinct cards. Multiple settings in one card should be visibly
grouped; avoid a long sequence of equally weighted labels and values.

| Control family | Treatment |
| --- | --- |
| Text field | Persistent label above; placeholder gives an example; bounded width; shared theme border/focus state |
| Secret field | Same geometry as text; masked value; label explains whether blank preserves the stored secret |
| Numeric value | Units in label/value; preserve range/step rules; avoid color as the only validation signal |
| Textarea | Label and help above; generous retained height; independent scroll; preserve newlines |
| Select/searchable select | Same row alignment as text; clear selected label; long catalogs scroll; searchable fonts remain searchable |
| Slider | Name and current value above; range/units/step understandable; track spans available form width |
| Boolean or segmented choice | Clear label and visible selected state; grouped choices 5px apart; keyboard focus distinct from selection |
| Checkbox | Visible label beside the native checkbox; focus indicator; preserve checked/unchecked semantics |
| Color entry/palette choice | Accessible text value/name; palette choices indicate selection; preserve hex validation and explicit save |
| Path/account/connect action | Explain destination or scope; group with its field; keep secondary details wrap-safe |
| Import editor | Labeled multi-line field with format/help; clear import action; preserve text when validation fails |
| Save/apply/test/reset action | Align with related fields; use precise verbs; separate destructive actions from routine choices |
| Read-only value | Text, rather than a disabled-looking fake editor; unavailable state explained |

Color settings currently use labeled hex text fields and palette selects. Boolean
settings currently include button-based choices. These are the actual native
forms, not a claim that dedicated color-picker or switch controls have already
been introduced. Choose a native switch/checkbox when a later component change
benefits from one, using the same label/state rules.

### Form states and behavior

- **Label:** visible at rest; never rely on the placeholder as the field name.
  Give the native control an accessible name that agrees with the label.
- **Help:** muted, wraps, and states an effect or constraint. Put it next to the
  relevant field rather than in a distant generic paragraph.
- **Error:** readable inline text identifying the field/problem; preserve input
  and show the recovery action. Avoid a permanent stale global error banner.
- **Disabled:** preserve label/value readability, disable mutation, and explain
  conditional availability (for example, transparency with an opaque surface).
- **Saving/loading:** retain layout and entered values; distinguish loading from
  an empty catalog. Disable only the affected controls where supported.
- **Commit:** preserve the current explicit-save or slider-release contract. A
  visual consistency pass must not silently change when settings are written.
- **Focus:** visible theme outline; tab order follows reading order. Enter in a
  preferences field belongs to that form, while chat Enter sends in the composer.
- **Responsive:** fields fill their card; help text wraps; long rows can stack.
  Vertical preference scrolling must reach every control without horizontal
  clipping. Sensitive values stay masked in screenshots and documentation.

Text and secret fields in the converted forms now have explicit accessible names.
The pinned slider API has no per-control label hook at these call sites; fuller
slider semantics, every native subpanel, and all density/narrow-screen combinations
remain review items. The initial token pass standardizes common
layout containers; it does not establish complete form-behavior parity.

The native extensions subpanel already includes checkbox/transport/import editor
patterns. Dedicated numeric/color/filesystem picker widgets remain a Tauri parity
gap. Native controls inherit shared themed 5px control and 10px large/card radii;
component-owned menu/tab geometry is deliberately retained.

## Adoption and review

This first pass adopts the shared spacing tokens in chats/composer, the expanded
sidebar/footer, General/Appearance/service/Extensions/Secrets preference forms,
and composer popups. Existing colors, alpha,
motion, tab treatment, and authority behavior are retained reference rules, not
claims that every interaction has passed complete parity testing. Browser, mail,
terminal, ancillary dialogs, vendor control geometry, and every narrow/density
combination require subsequent component-level review.

For each visual change, check the native rendering at the user's current setup,
then narrow panes, long labels, keyboard focus, scroll containment, and scale.
Reuse tokens; preserve deliberate native/Tauri exceptions. Record build, install,
and runtime evidence separately. Phone screenshots must remain below 512KiB
including inline-encoding overhead.

### Reference sources

- [`src/design.rs`](src/design.rs): palette, typography, scale, shared geometry.
- [`../design/reference.html`](../design/reference.html): established Tauri visual identity.
- [`../src/lib/components/AppSurface.svelte`](../src/lib/components/AppSurface.svelte):
  Tauri density, pane header, chat and composer treatment.
- [`src/shell.rs`](src/shell.rs), [`src/chat.rs`](src/chat.rs),
  [`src/composer_popup.rs`](src/composer_popup.rs): native component implementation.
- [`parity.json`](parity.json): implementation and verification boundaries.
