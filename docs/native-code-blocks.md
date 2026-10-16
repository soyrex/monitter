# Native chat code blocks

Markdown fenced and indented code blocks use the shared native TextView renderer:
12px regular monospace, 18px line height, a line-number gutter, language header,
exact-source Copy action, and horizontal scrolling. Vertical wheel events still
scroll the chat. Minimap previews use the same renderer.

Explicit fence languages take priority. Unlabelled blocks use a conservative,
UTF-8-safe 4KiB sample to identify clear language signatures. Ordinary prose is
not reclassified. Unsupported or ambiguous code remains readable plain code.

Unified diffs show red/green row shading. Hunk headers supply original/new file
line numbers; snippets without hunk headers use local block line numbers.

## Cost controls

- Presentation metadata is computed once per parsed block.
- Shaped width is cached by monospace font, and syntax spans by highlighter identity.
- Theme/grammar changes invalidate highlighting through the existing shared cache.
- Tree-sitter parsing has an 8ms budget and skips blocks larger than 128KiB.
- Parser instances use canonical language names and a bounded 32-entry cache.
- One text element renders the gutter; one canvas paints only visible diff rows.
- No model requests, background polling, webviews, or per-line UI entities.

Common enabled grammars: Rust, JavaScript, TypeScript/TSX, Python, Bash, JSON,
HTML, CSS, YAML, TOML, Go and diff. Additional grammars can be enabled explicitly.
