# Monitter native OpenUI output

Guide revision: 2026-10-21. Library: `monitter-openui-v1/monitter-native-v1/lang-core-1.0.1`.
Standard library source pin: `9823ec05d4b29bc78d101676dc16b44fe17f1830`.

Use this guide when the user would benefit from native interactive cards, forms,
tables or charts in a Monitter Fast desktop conversation. Emit OpenUI Lang inside
an exact `openui` Markdown fence in your assistant reply. Ordinary prose may
surround it. Other clients retain readable source; do not assume native rendering
on mobile or shared web clients. No package installation is needed.

Use `root = Stack([...])`. Components use positional arguments in the order below;
optional arguments may be omitted at the end. Variables such as `$name` bind local
presentation state. Use `TextContent` for prose; do not invent HTML, JSX or CSS
props. Keep output small and use only supported native components. Never describe
sample data as live project or provider data. Generated output cannot invoke tools
or grant permission.

## Presentation choice

When SYS-1 decision tools are available, follow their pre-final presentation
decision guidance before choosing OpenUI. Use at most one `jev_choose` call with
plain/cards/table/chart/form/abstain candidates. Skip trivial replies and explicit
format requests. Low-confidence, unavailable or abstaining decisions retain
ordinary Markdown. No decision grants permission to take actions.

## Reviewable interactions

A form prepares bounded response data. The host offers **Review response in
composer** to insert an editable draft in the originating chat. The user must
review and press Send explicitly. Do not promise automatic submission, purchases,
network requests or backend changes. Never ask for passwords, tokens or secrets.
Button action expressions are not executed. Interactivity becomes available once
the output is complete; incomplete streams retain the last usable tree.

## Example and common signatures

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

## Additional signatures

- `CardHeader(title?, subtitle?)`, `TextContent(text, size?)`, `TextCallout(variant?, title?, description?)`, `CodeBlock(language, codeString)`.
- `SelectItem(value, label)`, `TextArea(name, placeholder?, rows?, rules?, value?)`, `DatePicker(name, mode?, rules?, value?)`.
- `CheckBoxGroup(name, items, rules?, value?)`, `CheckBoxItem(label, description, name, defaultChecked?)`.
- `RadioGroup(name, items, defaultValue?, rules?, value?)`, `RadioItem(label, description, value)`.
- `LineChart(labels, series, variant?, xLabel?, yLabel?, height?)` and `AreaChart` use the same argument order. Variants: `linear`, `natural`, `step`.
- `PieChart(labels, values, variant?, appearance?)`: variants `pie` or `donut`; appearance `circular` or `semiCircular`.
- `ScatterChart(datasets, xLabel?, yLabel?)`, `ScatterSeries(name, points)`, `Point(x, y, z?)`.

Layout direction is `row` or `column`; gaps are `none`, `xs`, `s`, `m`, `l`, `xl`, `2xl`.
Card variants are `card`, `sunk`, `clear`. Text sizes are `small`, `default`, `large`,
`small-heavy`, `large-heavy`. Bar chart variants are `grouped` or `stacked`.
Rules include `required`, `email`, `url`, `numeric`, `min`, `max`, `minLength`,
`maxLength`. Keep label counts aligned with chart values and column row counts.

The outer surface is transparent and has no padding; cards have a 50% black
background tint (equivalent to black Multiply at 50%). Clear cards have no fill.
Text and controls retain full opacity. Styling follows the host theme and cannot
be overridden with model-supplied CSS.

## Safety and limits

Query and Mutation are unavailable. Input names and state keys are screened for secrets and prototype keys; password inputs are rejected. State is bounded to 16 KiB, depth six, 64 object keys, 32 array values, and 2 KiB per string. Output is limited to 64 KiB per OpenUI block, 300 components and depth 20. Tables allow eight columns and 32 rows; selects allow 32 items; charts allow eight series and 32 points. An incomplete stream retains the last usable tree. Field edits survive revisions for the same message/block identity. The Rust host independently sanitizes persisted state and draft payloads.

At most four blocks per message and eight active engines are supported. Form
drafts are capped at 8 KiB. Native Markdown displays selectable source text;
remote images display alt text placeholders. Sliders use step buttons, carousels
use page buttons, and charts have no hover tooltips. Prefer the documented core
signatures above when uncertain rather than guessing a component's props.

## Supported component groups

| Group | Components | Native behavior |
| --- | --- | --- |
| Layout | `Stack`, `Tabs`, `TabItem`, `Accordion`, `AccordionItem`, `Steps`, `StepsItem`, `Carousel`, `Separator`, `Modal` | Responsive stacks and cards; local tab/accordion selection; steps displayed in order; carousel page buttons; native modal window layers with dismiss controls and reactive `open` binding. |
| Content | `Card`, `CardHeader`, `TextContent`, `MarkDownRenderer`, `Callout`, `TextCallout`, `CodeBlock`, `InlineHeader` | Native themed text and containers. CodeBlock preserves the standard `language` and `codeString` props. Markdown is rendered as plain selectable text. |
| Forms | `Form`, `FormControl`, `Label`, `Input`, `TextArea`, `Select`, `SelectItem`, `DatePicker`, `Slider`, `CheckBoxGroup`, `CheckBoxItem`, `RadioGroup`, `RadioItem`, `SwitchGroup`, `SwitchItem`, `Chips`, `ChipItem`, `OptionCards`, `OptionCard` | Native editable fields and selections. Form buttons prepare a bounded draft for the composer; the user reviews it and sends explicitly. |
| Actions | `Button`, `Buttons`, `IconButton` | Button labels become draft actions; OpenUI `Action` expressions are not run. Icon names render as text labels where native icon mapping is unavailable. |
| Data | `Table`, `Col`, `EditableTable`, `EntityList`, `Tag`, `TagBlock`, `ListBlock`, `ListItem`, `Icon` | Bounded native tables, editable plain-text cell values, key/value lists and tags; component-valued read-only table cells are shown as text. Specialized select/date/url cell editors are not provided. |
| Charts | `BarChart`, `LineChart`, `AreaChart`, `HorizontalBarChart`, `RadarChart`, `PieChart`, `RadialChart`, `SingleStackedBarChart`, `ScatterChart`, `Series`, `Slice`, `ScatterSeries`, `Point` | Native Canvas geometry, axes and theme-based legends. Grouped/diverging stacked bars, line/area curves, pie/donut, concentric radial arcs, radar polygons and scatter/bubble plots. Hover tooltips are not provided. |
| Cards | `SnippetCardBlock`, `SnippetCardItem`, `OverviewCardBlock`, `OverviewCardItem`, `ContextCardBlock`, `ContextCardItem`, `CompositeCardBlock`, `CompositeCardItem`, `VisualCardBlock`, `VisualCardItem`, `Text`, `BoldText`, `IconText`, `ImageText`, `ImageTextLarge`, `MetricIndicatorInline`, `MetricIndicatorWithStrikethrough` | Native text, metrics and responsive card layouts. Image slots use descriptive placeholders. |
| Media | `Image`, `ImageBlock`, `ImageGallery` | Preserve alt text and captions in native placeholders. This renderer does not fetch or decode model supplied image URLs. |


`AgentInterface`, `ToolCall` and `ToolResult` belong to the host chat shell and
are not assistant output components.
