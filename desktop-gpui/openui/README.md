# OpenUI Lang → native NSH

Trusted adapter code interprets OpenUI's typed tree and renders native NSH views. Model output is never treated as JS/TSX, HTML, a module path, or a command. Query/Mutation are unavailable. Form submission prepares a bounded draft for the Rust host's composer review control; it never sends a message.

The allowlist is `Root`, `Stack`, `Row`, `Card`, `Heading`, `Text`, `Badge`, `Form`, `Input`, `Select`, `Checkbox`, `Button`, and `Table`. Arguments are positional in the following example:

```openui
$name = "Alex"
root = Root([
  Heading("Delivery preferences"),
  Card("Order", [Text("Review these details before preparing a response."), Badge("Draft")]),
  Form("reply", [
    Input("name", "Name", $name, "Enter a name"),
    Select("delivery", "Delivery", ["Pickup", "Shipping"], "Pickup"),
    Checkbox("confirmed", "Details checked", false)
  ], "Prepare response"),
  Table(["Item", "Quantity"], [["Sample", 2]])
])
```

`Root(children)`, `Stack(children)`, `Row(children)`, `Card(title, children)`, `Heading(content)`, `Text(content)`, `Badge(label)`, `Form(name, children, submitLabel?)`, `Input(name, label, value?, placeholder?)`, `Select(name, label, options, value?)`, `Checkbox(name, label, checked?)`, `Button(label)`, `Table(columns, rows)`. String/boolean field props support OpenUI `$binding` state. There are no password controls. Tables allow eight columns and 32 rows; selects allow 32 choices; trees allow 300 components and depth 20. Invalid final output reports a host fallback; a partial revision retains the last usable tree. Stable keyed native records retain input entities during streaming and state updates.

## Native bridge

Register `monitter.openui.bridge`. JS declares props `state`, `action`, `status`, `error`, `height` and event `Update` (handler `onUpdate`). On `status: "ready"`, emit:

```json
{"source":"root = Root([Text(\"Hello\")])","complete":true,"state":{},"messageId":"message UUID","blockId":"0","theme":{"dark":true,"accent":"#86a9ff","paper":"#171b22","panel":"#202630","ink":"#e6e9ef","muted":"#a4aebd","line":"#384353","fontFamily":".AppleSystemUIFont","fontSize":14}}
```

Retain the same AppView across revisions. `messageId` + `blockId` identify state; switching identity resets parser/store, while subsequent `state` echoes for the same identity do not overwrite edits. `state` is initially null (host must ignore it); user edits emit a JSON object snapshot: `$binding` keys plus plain form/field values. Initial hydration never emits an action or a state save. `action: null` clears any pending draft on an Update. State snapshots are bounded to 16 KiB, nesting depth four, 64 keys/container, 32 array entries, and 2 KiB/string; secret-like/prototype keys are excluded. Rust must independently sanitize and persist by block/library identity (`monitter-native-v1`).

User submission sets `action: {type:"draft", text:string, id:string}`. The monotonically increasing id permits deduplication. The host validates bounds, exposes its own review control, inserts into the editable composer only on user approval, and preserves explicit Send. Source never supplies a raw action or command.

`status` is `ready`, `incomplete` (no usable tree), `rendered`, or `error`. `error` is a bounded string. `height` is measured intrinsic content height, clamped to 80–100000 points. Output contributes its full measured height to the host transcript; the transcript owns vertical scrolling, with no nested Scroll. Rows wrap and cards use a flexible 280-point preferred width bounded by the available pane width. Theme CSS colors come from the host.

## Reproducibility and build

`npm ci --ignore-scripts` installs pinned build dependencies; no script installs or fetches during application builds. `npm run vendor` regenerates `vendor/core.js` from the ignored official clone at `../../artifacts/openui-nsh/upstream`, requiring commit `9823ec05d4b29bc78d101676dc16b44fe17f1830`. This bundles the official framework-independent parser/library/evaluator/store, Zod 4.4.3, and the message protocol, with licenses and a SHA-256 provenance record. Server-only `ci-info` discovery is replaced by an inert module. NSH grants remain empty.

Use NSH's `compile_module("renderer.tsx", source, &[])` first. Then `node desktop-gpui/openui/bundle.mjs /absolute/renderer.compiled.js /absolute/openui-renderer.js` bundles imports while leaving `nsh`, `nsh/*`, and `nsh:rt` native. Do not run this TSX through React's JSX transform. The runtime uses only checked-in trusted JS and the configured empty NSH capabilities.

`npm test` verifies streaming, field identity/state restore, replay envelopes, unsupported tools/components, and state redaction with the actual vendored official parser. Native focus, scrolling, theme, host fallback, and composer review require the running Fast UI target.
