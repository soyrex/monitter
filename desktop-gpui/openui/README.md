# OpenUI Lang on native GPUI Fast

This adapter uses the project's pinned OpenUI Lang 1.0.1 parser and library format, then reconciles its typed component references into trusted NSH renderers hosted in GPUI Fast. It does not execute model-authored JSX, HTML, JavaScript, URLs, commands, or OpenUI action expressions. The native library root is `Stack`; component props and their positional order follow the OpenUI library pinned at `9823ec05d4b29bc78d101676dc16b44fe17f1830`.

## Native core coverage

The native registry covers the components in the pinned OpenUI Lang standard library, using trusted NSH renderers hosted in GPUI Fast. Component references and positional props follow the pinned schemas. Native presentation is not React visual parity; the table lists the deliberate limitations:

| Group | Components | Native behavior |
| --- | --- | --- |
| Layout | `Stack`, `Tabs`, `TabItem`, `Accordion`, `AccordionItem`, `Steps`, `StepsItem`, `Carousel`, `Separator`, `Modal` | Responsive stacks and cards; local tab/accordion selection; steps displayed in order; carousel page buttons; native modal window layers with dismiss controls and reactive `open` binding. |
| Content | `Card`, `CardHeader`, `TextContent`, `MarkDownRenderer`, `Callout`, `TextCallout`, `CodeBlock`, `InlineHeader` | Native themed text and containers. CodeBlock preserves the standard `language` and `codeString` props. Markdown is rendered as plain selectable text. |
| Forms | `Form`, `FormControl`, `Label`, `Input`, `TextArea`, `Select`, `SelectItem`, `DatePicker`, `Slider`, `CheckBoxGroup`, `CheckBoxItem`, `RadioGroup`, `RadioItem`, `SwitchGroup`, `SwitchItem`, `Chips`, `ChipItem`, `OptionCards`, `OptionCard` | Native editable fields and selections. Form buttons prepare a bounded draft for the composer; the user reviews it and sends explicitly. |
| Actions | `Button`, `Buttons`, `IconButton` | Button labels become draft actions; OpenUI `Action` expressions are not run. Icon names render as text labels where native icon mapping is unavailable. |
| Data | `Table`, `Col`, `EditableTable`, `EntityList`, `Tag`, `TagBlock`, `ListBlock`, `ListItem`, `Icon` | Bounded native tables, editable plain-text cell values, key/value lists and tags; component-valued read-only table cells are shown as text. Specialized select/date/url cell editors are not provided. |
| Charts | `BarChart`, `LineChart`, `AreaChart`, `HorizontalBarChart`, `RadarChart`, `PieChart`, `RadialChart`, `SingleStackedBarChart`, `ScatterChart`, `Series`, `Slice`, `ScatterSeries`, `Point` | Native Canvas geometry, axes and theme-based legends. Grouped/diverging stacked bars, line/area curves, pie/donut, concentric radial arcs, radar polygons and scatter/bubble plots. Hover tooltips are not provided. |
| Cards | `SnippetCardBlock`, `SnippetCardItem`, `OverviewCardBlock`, `OverviewCardItem`, `ContextCardBlock`, `ContextCardItem`, `CompositeCardBlock`, `CompositeCardItem`, `VisualCardBlock`, `VisualCardItem`, `Text`, `BoldText`, `IconText`, `ImageText`, `ImageTextLarge`, `MetricIndicatorInline`, `MetricIndicatorWithStrikethrough` | Native text, metrics and responsive card layouts. Image slots use descriptive placeholders. |
| Media | `Image`, `ImageBlock`, `ImageGallery` | `Image` and `ImageBlock` display host-approved public media. Other sources remain placeholders. Gallery images retain placeholders. |

`AgentInterface`, `ToolCall`, and `ToolResult` are host chat-shell components rather than assistant output components, so they remain outside this renderer. All other registry names have explicit native render paths. Remote media are placeholders because the output engine has no network/filesystem grants. `Image` and `ImageBlock` accept `media:<64 lowercase hex characters>.jpg|png|webp` when the host finds a regular raster file under `~/Library/Application Support/Monitter/openui-public-media` (macOS) or the corresponding XDG data directory. The host checks the file's canonical location, size and dimensions before passing its path to the renderer. Only put public, user-approved display assets there. Markdown currently remains plain selectable text; swipe gestures, specialized table editors and chart tooltips remain unmatched as detailed below.

Some components are intentionally simplified by available native primitives: `TextArea` uses native multiline editing; `DatePicker` uses a month calendar with single/range selection and serialized ISO date values; `Slider` changes by step buttons; `Carousel` uses page buttons without swipe navigation; `Modal` uses NSH’s native dialog layer with close, Escape and backdrop dismissal; and `Callout` remains visible until its bound state changes. `MarkDownRenderer` displays its source as selectable text. Remote image URLs remain placeholders. Charts draw native Canvas geometry for grouped/stacked bars, lines/areas, pie/donut, radial, radar and scatter/bubbles. Natural curves use smooth Bézier interpolation, dense point markers are sampled, and category legends show the first eight entries. Pie/radial charts use absolute magnitudes for signed values and preserve signed legend text. Chart recording is bounded to 8192 encoded values per canvas (the tested maximum is 2954). Form submission enforces required, email, URL, numeric, range, and length rules. Regex `pattern` rules accept only anchored expressions with at most one repetition operator; groups, alternation, and other constructs are rejected to keep untrusted validation bounded. Non-password `Input` types use a plain text control. These components preserve their OpenUI wire schemas and state values where practical, but they do not claim visual or interaction parity with React.

## Examples

```openui
$name = "Alex"
root = Stack([
  Card([CardHeader("Delivery preferences"), TextContent("Review your choices before preparing a response.")]),
  Form("reply", Buttons([Button("Prepare response")]), [
    FormControl("Name", Input("name", "Enter a name", "text", {}, $name)),
    FormControl("Delivery", Select("delivery", [SelectItem("pickup", "Pickup"), SelectItem("ship", "Shipping")], "Choose delivery"))
  ]),
  Table([Col("Item", ["Sample"]), Col("Quantity", [2], "number")])
])
```

Core positional signatures are `Stack(children, direction?, gap?, align?, justify?, wrap?)`, `Card(children, variant?, direction?, gap?, align?, justify?, wrap?)`, `Tabs(items)`, `TabItem(value, trigger, content)`, `Accordion(items)`, `AccordionItem(value, trigger, content)`, `Form(name, buttons, fields?)`, `FormControl(label, input, hint?)`, `Input(name, placeholder?, type?, rules?, value?)`, `Select(name, items, placeholder?, rules?, value?, size?)`, `Button(label, action?, variant?, type?, size?)`, `Buttons(buttons, direction?)`, `Table(columns)`, `Col(label, data, type?)`, `BarChart(labels, series, variant?, xLabel?, yLabel?, height?)`, and `Series(category, values)`. Nested content uses references such as `TabItem` content arrays, `SelectItem` option references, and `Col` table references; the adapter reconciles these through streaming updates.

## Safety and limits

Query and Mutation are unavailable. Input names and state keys are screened for secrets and prototype keys; password inputs are rejected. State is bounded to 16 KiB, depth six, 64 object keys, 32 array values, and 2 KiB per string. Output is limited to 64 KiB per OpenUI block, 300 components and depth 20. Tables allow eight columns and 32 rows; selects allow 32 items; charts allow eight series and 32 points. An incomplete stream retains the last usable tree. Field edits survive revisions for the same message/block identity. The Rust host independently sanitizes persisted state and draft payloads.

## Native bridge and build

The renderer is compiled by NSH's `compile_module("renderer.tsx", source, &[])`, then bundled with `node desktop-gpui/openui/bundle.mjs <compiled.js> <openui-renderer.js>`. Do not run the TSX through React's JSX transform. The NSH capability grant remains empty and browser-only storage is replaced with an ephemeral backend.

The runtime emits `status` (`ready`, `incomplete`, `rendered`, or `error`), bounded error text, intrinsic height, sanitized state, and a `draft` action. Rust treats the action as a reviewable composer draft only; it never sends automatically. See `openui_output.rs` for the message parser, persistence identity, fallback, and final bounds.

`status` is `ready`, `incomplete` (no usable tree), `rendered`, or `error`. `error` is a bounded string. `height` is measured intrinsic content height, clamped to 80–100000 points. Output contributes its full measured height to the host transcript; the transcript owns vertical scrolling, with no nested Scroll. Rows wrap and cards use a flexible 280-point preferred width bounded by the available pane width. Theme CSS colors come from the host.

`npm ci --ignore-scripts` installs pinned build dependencies. `npm run vendor` regenerates `vendor/core.js` from the ignored official clone at `../../artifacts/openui-nsh/upstream`, requiring the pinned commit and recording provenance. Application builds use the checked-in trusted bundle and perform no download/install step.

## Verification receipt (2026-10-21)

On the isolated `feat/openui-native-standard-components` branch, all 30 JavaScript tests and all 20 native OpenUI tests passed. Native build run `11e48f53-9d02-49b1-b708-c306dc093fbe` exited zero. Coverage includes the pinned standard-schema fixtures, calendar range values, modal state, chart Canvas geometry, retained table inputs and streamed choices. The real transcript regression checks native Input painting, edits through streaming/completion/cache replay, responsive card wrapping and host wheel scrolling over native output.

The NSH surface and hosted-component hitboxes preserve mouse isolation while allowing unhandled wheel events to reach the host list. Native scroll handlers still receive the event first and can consume it.

The shared app cache can reuse a build script from another worktree with an older embedded renderer. The build script now reads renderer source from its runtime manifest directory. When switching between this branch and an older canonical checkout sharing the same target directory, force regeneration by touching `desktop-gpui/build.rs` before the coordinated build. The successful receipt followed that regeneration; dependency caches were retained. Disk finished with approximately 19 GiB free, with no cleanup performed in this work.

These results verify this documented native subset, including the approximations above. The isolated port was subsequently merged into local main. It has not been packaged, installed or activated as part of this delivery.
