# HTML reports in Monitter

Before generating a report, call the read-only collaboration MCP tool
`get_report_styles` with `{}`. It returns `version`, `capturedAt` (Unix
milliseconds), `theme`, `css`, `starterHtml`, and `usage`. The equivalent CSS
resource is `monitter://reports/theme.css`, discovered with `resources/list`
and read with `resources/read`. Both routes use the same caller authorization
as the tool: the originating task must be running and its agent must have
collaboration enabled. Read-only task sandboxes can retrieve styles.

The native GUI publishes its effective light or dark palette, accent,
font families and scaled font size to the local server. The server keeps only
the latest validated publication in memory. If no GUI has published a theme,
the tool/resource returns an unavailable error explaining how to retry; it does
not invent a theme or restore an old one from disk.

When a newer GUI connects to an older resident server without the publication
method, it silently disables publication for that server instance. It retries
with a replacement server; genuine transport and validation errors remain visible.
This does not upgrade or restart the server, and report tools remain unavailable
until a supporting backend is running.

Start from `starterHtml`, replace the title and report content, and preserve
its viewport meta tag. Embed the returned CSS in a `<style>` element, use
`class="monitter-report"` on the body or report root, and use
`<main class="report-container">` for responsive width and padding.
`report-header` and `report-footer` provide the page framing. These classes
are scoped beneath `.monitter-report`:

- `report-grid`, `report-card`, and `report-metric` for summaries and metrics.
- `report-eyebrow`, `report-meta`, and `report-badge` for secondary information.
- `report-callout` for a highlighted observation.
- `report-table-wrap` around wide tables. Give the wrapper `tabindex="0"` and
  an `aria-label` describing the table so keyboard users can scroll it.
- `report-numeric` for numeric table cells, `report-sr-only` for accessible
  descriptions, and `report-no-print` for controls omitted from printed output.
- `report-layout`, `report-sidebar`, and `report-toc` for optional contents
  navigation, with `report-skip-link` for keyboard access to the main content.

For a long standalone report, wrap the sidebar and main container as siblings:

```html
<body class="monitter-report">
  <a class="report-skip-link" href="#report-main">Skip to report</a>
  <div class="report-layout">
    <aside class="report-sidebar">
      <p class="report-eyebrow">Report contents</p>
      <nav class="report-toc" aria-label="Report sections">
        <a href="#overview">Overview</a>
        <a href="#findings">Findings</a>
        <a href="#evidence">Evidence</a>
      </nav>
    </aside>
    <main class="report-container" id="report-main" tabindex="-1">
      <header class="report-header" id="overview"><h1>Report title</h1></header>
      <section aria-labelledby="findings"><h2 id="findings">Findings</h2></section>
      <section aria-labelledby="evidence"><h2 id="evidence">Evidence</h2></section>
    </main>
  </div>
</body>
```

At viewport widths of at least `64rem` (normally 1024px), the sidebar is fixed
to the inline start of the viewport and scrolls independently if its contents
exceed the window height. The layout reserves `16rem` for it so the report does
not overlap. Below that breakpoint, the sidebar flows above the main content
and its links wrap into pills. It is not sticky on small screens, so a long
contents list does not cover the report. Printing hides the sidebar and skip
link and restores the full content width. Use the layout once per standalone
document; its fixed sidebar is relative to the viewport.

The links use native fragment navigation and visible keyboard focus; no script
is required. `aria-current="location"` optionally highlights the current link
when the report supplies that state; CSS does not track scrolling. A standalone
`nav.report-toc` without the sidebar/layout wrapper stays a wrapping pill set
at every width. Leave out the layout classes for the original single-column
report format. The existing `starterHtml` remains the simple default.

The component CSS is compiled into the server from `src-tauri/src/report.css`.
Updating source requires rebuilding and restarting the backend before
`get_report_styles` and `monitter://reports/theme.css` return the new components.
Previously saved HTML keeps its embedded snapshot until it is explicitly updated.

Use semantic headings, sections, lists, figures, captions, tables, and native
`details` elements. Prefer the grid to fixed pixel widths. Long tables and code
blocks scroll within their own containers; print styles produce a readable
light document. Escape user-provided report content as HTML.

Fonts use the published local family, then system fallbacks. No external font
service is called and no font files are embedded in the report. A GUI-embedded
font may be unavailable to the browser unless installed on the host.

Save the completed document to an existing `.html` or `.htm` file and call
`browser_open` with a stable `request_id` and the absolute path or file URI to
show it beside the originating chat. Remote tasks must use an HTTP(S) URL.
The queued result does not prove that the page opened or rendered; inspect the
actual browser pane when verification is required.

The CSS is a snapshot of the theme at `capturedAt`. Saved reports retain it
when Monitter's theme later changes. Retrieve styles again before generating a
new report when you need the current appearance. Reports contain presentation
metadata only; the API grants no browser control or theme-writing authority.
