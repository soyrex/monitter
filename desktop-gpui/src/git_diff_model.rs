//! Pure unified-diff parsing and bounded display projection. Highlight ranges are UTF-8
//! byte ranges into `DiffCell::text`. Fold IDs are stable within one document.
use std::{collections::HashSet, ops::Range};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LineKind {
    Context,
    Addition,
    Deletion,
}
#[derive(Clone, Debug)]
pub(super) struct DiffLine {
    pub kind: LineKind,
    pub old_number: Option<usize>,
    pub new_number: Option<usize>,
    pub text: String,
    pub no_newline: bool,
}
#[derive(Clone, Debug)]
pub(super) struct DiffHunk {
    pub header: String,
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub function: String,
    pub lines: Vec<DiffLine>,
}
#[derive(Clone, Debug, Default)]
pub(super) struct DiffDocument {
    pub hunks: Vec<DiffHunk>,
    pub additions: usize,
    pub deletions: usize,
}
#[derive(Clone, Debug)]
pub(super) struct DiffCell {
    pub number: Option<usize>,
    pub text: String,
    pub highlights: Vec<Range<usize>>,
    pub no_newline: bool,
}
#[derive(Clone, Debug)]
pub(super) enum DiffRow {
    Hunk {
        index: usize,
        header: String,
    },
    Code {
        old: Option<DiffCell>,
        new: Option<DiffCell>,
        kind: LineKind,
    },
    Fold {
        id: usize,
        count: usize,
    },
    Note(String),
}
#[derive(Clone, Debug)]
pub(super) struct DiffOptions {
    pub split: bool,
    pub show_whitespace: bool,
    pub expanded_folds: HashSet<usize>,
    pub visible_hunks: usize,
    pub row_limit: usize,
}
impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            split: false,
            show_whitespace: false,
            expanded_folds: HashSet::new(),
            visible_hunks: 2,
            row_limit: 400,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub(super) struct DiffProjection {
    pub rows: Vec<DiffRow>,
    pub hidden_hunks: usize,
    pub hidden_rows: usize,
    pub hidden_whitespace: usize,
}

fn range(s: &str, sign: char) -> Option<(usize, usize)> {
    let s = s.strip_prefix(sign)?;
    let mut p = s.split(',');
    let start = p.next()?.parse().ok()?;
    let count = match p.next() {
        Some(n) => n.parse().ok()?,
        None => 1,
    };
    if p.next().is_some() {
        return None;
    }
    Some((start, count))
}
fn header(s: &str) -> Option<DiffHunk> {
    let rest = s.strip_prefix("@@ ")?;
    let (ranges, function) = rest.split_once(" @@")?;
    let mut p = ranges.split_whitespace();
    let (old_start, old_count) = range(p.next()?, '-')?;
    let (new_start, new_count) = range(p.next()?, '+')?;
    if p.next().is_some() {
        return None;
    }
    Some(DiffHunk {
        header: s.into(),
        old_start,
        old_count,
        new_start,
        new_count,
        function: function.trim_start().into(),
        lines: vec![],
    })
}
impl DiffDocument {
    pub fn parse(input: &str) -> Self {
        let mut doc = Self::default();
        let (mut old, mut new, mut old_left, mut new_left) = (0, 0, 0, 0);
        for raw in input.lines() {
            let raw = raw.strip_suffix('\r').unwrap_or(raw);
            if raw.starts_with("diff --git ")
                || raw.starts_with("diff --cc ")
                || raw.starts_with("diff --combined ")
            {
                old_left = 0;
                new_left = 0;
                continue;
            }
            if let Some(h) = header(raw) {
                old = h.old_start;
                new = h.new_start;
                old_left = h.old_count;
                new_left = h.new_count;
                doc.hunks.push(h);
                continue;
            }
            let Some(h) = doc.hunks.last_mut() else {
                continue;
            };
            if raw == "\\ No newline at end of file" {
                if let Some(l) = h.lines.last_mut() {
                    l.no_newline = true;
                }
                continue;
            }
            let Some(sign) = raw.as_bytes().first() else {
                continue;
            };
            let (kind, on, nn) = match *sign {
                b' ' if old_left > 0 && new_left > 0 => {
                    old_left -= 1;
                    new_left -= 1;
                    let n = (LineKind::Context, Some(old), Some(new));
                    old = old.saturating_add(1);
                    new = new.saturating_add(1);
                    n
                }
                b'-' if old_left > 0 => {
                    old_left -= 1;
                    doc.deletions += 1;
                    let n = (LineKind::Deletion, Some(old), None);
                    old = old.saturating_add(1);
                    n
                }
                b'+' if new_left > 0 => {
                    new_left -= 1;
                    doc.additions += 1;
                    let n = (LineKind::Addition, None, Some(new));
                    new = new.saturating_add(1);
                    n
                }
                _ => continue,
            };
            h.lines.push(DiffLine {
                kind,
                old_number: on,
                new_number: nn,
                text: raw[1..].into(),
                no_newline: false,
            });
        }
        doc
    }
    /// Project at most `visible_hunks` hunks and `row_limit` rows; increasing either
    /// reveals more. Expanded fold IDs reveal their complete context run.
    pub fn project(&self, options: &DiffOptions) -> DiffProjection {
        let mut result = DiffProjection {
            hidden_hunks: self.hunks.len().saturating_sub(options.visible_hunks),
            ..Default::default()
        };
        let mut rows = Vec::new();
        let mut fold_id = 0;
        for (hi, h) in self.hunks.iter().enumerate().take(options.visible_hunks) {
            rows.push(DiffRow::Hunk {
                index: hi,
                header: h.header.clone(),
            });
            let mut i = 0;
            while i < h.lines.len() {
                if h.lines[i].kind == LineKind::Context {
                    let start = i;
                    while i < h.lines.len() && h.lines[i].kind == LineKind::Context {
                        i += 1;
                    }
                    let len = i - start;
                    let id = fold_id;
                    fold_id += 1;
                    if len > 6 && !options.expanded_folds.contains(&id) {
                        for l in &h.lines[start..start + 3] {
                            rows.push(context(l));
                        }
                        rows.push(DiffRow::Fold { id, count: len - 6 });
                        for l in &h.lines[i - 3..i] {
                            rows.push(context(l));
                        }
                    } else {
                        for l in &h.lines[start..i] {
                            rows.push(context(l));
                        }
                    }
                    continue;
                }
                let start = i;
                while i < h.lines.len() && h.lines[i].kind != LineKind::Context {
                    i += 1;
                }
                let removed: Vec<_> = h.lines[start..i]
                    .iter()
                    .filter(|l| l.kind == LineKind::Deletion)
                    .collect();
                let added: Vec<_> = h.lines[start..i]
                    .iter()
                    .filter(|l| l.kind == LineKind::Addition)
                    .collect();
                let mut unified_additions = Vec::new();
                for k in 0..removed.len().max(added.len()) {
                    let a = removed.get(k).copied();
                    let b = added.get(k).copied();
                    if let (Some(a), Some(b)) = (a, b) {
                        if !options.show_whitespace
                            && whitespace_equal(&a.text, &b.text)
                            && a.text != b.text
                            && a.no_newline == b.no_newline
                        {
                            result.hidden_whitespace += 1;
                            continue;
                        }
                    }
                    let (ah, bh) = match (a, b) {
                        (Some(a), Some(b)) => inline_ranges(&a.text, &b.text),
                        _ => (vec![], vec![]),
                    };
                    let ac = a.map(|l| cell(l, true, ah));
                    let bc = b.map(|l| cell(l, false, bh));
                    if options.split {
                        rows.push(DiffRow::Code {
                            old: ac,
                            new: bc,
                            kind: if a.is_some() {
                                LineKind::Deletion
                            } else {
                                LineKind::Addition
                            },
                        });
                    } else {
                        if ac.is_some() {
                            rows.push(DiffRow::Code {
                                old: ac,
                                new: None,
                                kind: LineKind::Deletion,
                            });
                        }
                        if bc.is_some() {
                            unified_additions.push(DiffRow::Code {
                                old: None,
                                new: bc,
                                kind: LineKind::Addition,
                            });
                        }
                    }
                }
                rows.extend(unified_additions);
            }
        }
        result.hidden_rows = rows.len().saturating_sub(options.row_limit);
        rows.truncate(options.row_limit);
        result.rows = rows;
        result
    }
}
fn cell(l: &DiffLine, old: bool, highlights: Vec<Range<usize>>) -> DiffCell {
    DiffCell {
        number: if old { l.old_number } else { l.new_number },
        text: l.text.clone(),
        highlights,
        no_newline: l.no_newline,
    }
}
fn context(l: &DiffLine) -> DiffRow {
    DiffRow::Code {
        old: Some(cell(l, true, vec![])),
        new: Some(cell(l, false, vec![])),
        kind: LineKind::Context,
    }
}
fn whitespace_equal(a: &str, b: &str) -> bool {
    a.chars()
        .filter(|c| !c.is_whitespace())
        .eq(b.chars().filter(|c| !c.is_whitespace()))
}
/// Linear, allocation-bounded prefix/suffix comparison. Token boundaries expand
/// the changed span to complete words; no quadratic LCS on minified lines.
fn inline_ranges(a: &str, b: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let mut prefix = 0;
    for (x, y) in a.chars().zip(b.chars()) {
        if x != y {
            break;
        }
        prefix += x.len_utf8();
    }
    let mut asuf = a.len();
    let mut bsuf = b.len();
    for (x, y) in a[prefix..].chars().rev().zip(b[prefix..].chars().rev()) {
        if x != y {
            break;
        }
        asuf -= x.len_utf8();
        bsuf -= y.len_utf8();
    }
    fn span(s: &str, mut start: usize, mut end: usize) -> Vec<Range<usize>> {
        if start == end {
            return vec![];
        }
        while start > 0 {
            let c = s[..start].chars().next_back().unwrap();
            if !c.is_alphanumeric() && c != '_' {
                break;
            }
            start -= c.len_utf8();
        }
        while end < s.len() {
            let c = s[end..].chars().next().unwrap();
            if !c.is_alphanumeric() && c != '_' {
                break;
            }
            end += c.len_utf8();
        }
        vec![start..end]
    }
    (span(a, prefix, asuf), span(b, prefix, bsuf))
}
pub(super) fn classify_path(path: &str) -> Option<&'static str> {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    if matches!(
        name,
        "package-lock.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "cargo.lock"
            | "poetry.lock"
            | "composer.lock"
            | "gemfile.lock"
            | "bun.lock"
            | "bun.lockb"
    ) {
        Some("Dependency lock file")
    } else if name.contains(".min.") || name.ends_with(".map") {
        Some("Minified or bundled output")
    } else if lower.split('/').any(|p| {
        matches!(
            p,
            "generated" | "__generated__" | "dist" | "build" | "vendor" | "node_modules"
        )
    }) || name.ends_with(".generated.rs")
        || name.ends_with(".g.dart")
    {
        Some("Generated or vendored file")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_headers_and_numbers() {
        let d = DiffDocument::parse(
            "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -3,2 +3,2 @@ fn hello\n same\n-old\n+new\n\\ No newline at end of file\n",
        );
        assert_eq!((d.additions, d.deletions), (1, 1));
        let h = &d.hunks[0];
        assert_eq!(h.function, "fn hello");
        assert_eq!(h.lines[2].new_number, Some(4));
        assert!(h.lines[2].no_newline);
    }
    #[test]
    fn unicode_and_bounds() {
        let (a, b) = inline_ranges("let café = 🦀;", "let cafés = 🐟;");
        for r in a {
            assert!("let café = 🦀;".get(r).is_some());
        }
        for r in b {
            assert!("let cafés = 🐟;".get(r).is_some());
        }
        let s = "a".repeat(100_000);
        let (a, b) = inline_ranges(&s, &format!("{s}b"));
        assert!(a.is_empty());
        assert_eq!(b.len(), 1);
    }
    #[test]
    fn split_and_whitespace() {
        let d = DiffDocument::parse("@@ -1,2 +1,3 @@\n-a b\n-old\n+a  b\n+new\n+extra\n");
        let mut o = DiffOptions {
            split: true,
            ..Default::default()
        };
        let p = d.project(&o);
        assert_eq!(p.hidden_whitespace, 1);
        assert_eq!(p.rows.len(), 3);
        o.show_whitespace = true;
        assert_eq!(d.project(&o).rows.len(), 4);
    }
    #[test]
    fn folds_and_caps() {
        let mut s = "@@ -1,10 +1,10 @@\n".to_string();
        for _ in 0..10 {
            s.push_str(" x\n");
        }
        let d = DiffDocument::parse(&s);
        let mut o = DiffOptions::default();
        assert!(matches!(
            d.project(&o).rows[4],
            DiffRow::Fold { count: 4, .. }
        ));
        o.expanded_folds.insert(0);
        o.row_limit = 5;
        let p = d.project(&o);
        assert_eq!(p.rows.len(), 5);
        assert_eq!(p.hidden_rows, 6);
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    #[test]
    fn marker_like_content_is_code_within_ranges() {
        let d = DiffDocument::parse(
            "--- a/a\n+++ b/a\n@@ -1 +1 @@\n--- content\n+++ content\ndiff --git a/b b/b\n--- a/b\n+++ b/b\n@@ -0,0 +1 @@\n+next\n",
        );
        assert_eq!((d.deletions, d.additions), (1, 2));
        assert_eq!(d.hunks[0].lines[0].text, "-- content");
        assert_eq!(d.hunks[0].lines[1].text, "++ content");
        assert_eq!(d.hunks[1].lines[0].new_number, Some(1));
    }
    #[test]
    fn truncated_file_stops_at_next_file_header() {
        let d = DiffDocument::parse(
            "@@ -1,9 +1,9 @@\n-old\n+new\ndiff --git a/b b/b\n--- a/b\n+++ b/b\n@@ -7 +8 @@ other\n-last\n+next\n",
        );
        assert_eq!((d.deletions, d.additions), (2, 2));
        assert_eq!(d.hunks[0].lines.len(), 2);
        assert_eq!(d.hunks[1].lines[0].old_number, Some(7));
    }
    #[test]
    fn rename_without_hunks_is_empty() {
        let d = DiffDocument::parse(
            "diff --git a/old b/new\nsimilarity index 100%\nrename from old\nrename to new\n",
        );
        assert!(d.hunks.is_empty());
        assert!(d.project(&DiffOptions::default()).rows.is_empty());
    }
    #[test]
    fn unified_keeps_deletion_and_addition_blocks() {
        let d = DiffDocument::parse(
            "@@ -1,2 +1,2 @@\n-first old\n-second old\n+first new\n+second new\n",
        );
        let p = d.project(&DiffOptions::default());
        let kinds: Vec<_> = p
            .rows
            .iter()
            .filter_map(|r| match r {
                DiffRow::Code { kind, .. } => Some(kind.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                LineKind::Deletion,
                LineKind::Deletion,
                LineKind::Addition,
                LineKind::Addition
            ]
        );
        let split = d.project(&DiffOptions {
            split: true,
            ..Default::default()
        });
        assert!(
            matches!(&split.rows[2], DiffRow::Code {old:Some(a), new:Some(b), ..} if a.text == "second old" && b.text == "second new")
        );
    }
    #[test]
    fn whitespace_filter_retains_semantic_and_newline_changes() {
        let d = DiffDocument::parse("@@ -1,2 +1,2 @@\n-a b\n-value = 1\n+a  b\n+value = 2\n");
        let p = d.project(&DiffOptions::default());
        assert_eq!(p.hidden_whitespace, 1);
        assert_eq!(p.rows.len(), 3);
        assert_eq!((d.additions, d.deletions), (2, 2));
        let newline =
            DiffDocument::parse("@@ -1 +1 @@\n-a b\n\\ No newline at end of file\n+a  b\n");
        assert_eq!(
            newline.project(&DiffOptions::default()).hidden_whitespace,
            0
        );
    }
    #[test]
    fn hunk_limit_and_zero_count_ranges() {
        let d =
            DiffDocument::parse("@@ -0,0 +1 @@\n+one\n@@ -4 +0,0 @@\n-gone\n@@ -9 +9 @@\n same\n");
        let p = d.project(&DiffOptions::default());
        assert_eq!(p.hidden_hunks, 1);
        assert_eq!(d.hunks[1].lines[0].new_number, None);
    }
}
