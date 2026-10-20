# OpenUI output in Fast

Fast supports an explicit `openui` Markdown fence in assistant messages. Ordinary prose and Markdown may surround the fence. Older clients show readable source. The adapter uses the official pinned OpenUI Lang parser/evaluator and a fixed NSH renderer; model-provided JavaScript, HTML, TSX and module paths are never loaded.

Example:

````markdown
```openui
$name = "Alex"
root = Root([
  Heading("Delivery preferences"),
  Text("Review the details before preparing a response."),
  Form("reply", [
    Input("name", "Name", $name, "Enter a name"),
    Select("delivery", "Delivery", ["Pickup", "Shipping"], "Pickup"),
    Checkbox("confirmed", "Details checked", false)
  ], "Prepare response")
])
```
````

Form submission exposes a separate Monitter **Review response in composer** button. This appends an editable draft to the originating chat and focuses its composer. It does not send it. Existing backend send and permission checks remain authoritative.

Values persist as bounded local presentation state keyed by library version, task, message and block ordinal. Password controls and sensitive field names are unsupported; do not put secrets in generated forms. NSH receives no network, process, filesystem or clipboard grants; the fixed renderer exposes no storage API to source. Unsupported components and malformed final documents show the original source. At most four blocks per message and eight active engines are supported; old hidden surfaces can be recreated from saved values.

## Building

From `desktop-gpui/openui`, run `npm ci --ignore-scripts` once using the checked-in lockfile. Cargo's build script compiles the trusted TSX with NSH, then bundles the adapter and checked-in official core using Node/esbuild. It never installs packages or fetches upstream automatically. `npm test` exercises streaming, state, bounded actions and unsupported output. `npm run vendor` is a maintainer operation requiring the exact clean upstream commit recorded in provenance.

Native tests use the `openui` filter. Build/test success is separate from actual Fast interaction, streaming, scrolling, draft review and restart verification; runtime evidence is recorded separately when collected.
