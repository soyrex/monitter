//! Immutable presentation metadata, built once per parsed block, never per paint.
use gpui::SharedString;

#[derive(Debug, Clone)]
pub(super) struct CodePresentation {
    pub language: Option<SharedString>,
    pub gutter: SharedString,
    pub changes: Vec<(usize, bool)>,
    pub gutter_columns: usize,
}

pub(super) fn inferred_language(code: &str) -> Option<&'static str> {
    // Inference is deliberately narrow and bounded. Unknown code stays plain.
    if code.len() > 128 * 1024 {
        return None;
    }
    let mut end = code.len().min(4096);
    while !code.is_char_boundary(end) {
        end -= 1;
    }
    let sample = &code[..end];
    let text = sample.trim_start();
    if (text.starts_with("diff --git ") || text.starts_with("--- "))
        && sample.lines().any(|line| hunk_start(line).is_some())
    {
        return Some("diff");
    }
    if text.starts_with("#!/bin/bash") || text.starts_with("#!/usr/bin/env bash") {
        return Some("bash");
    }
    if text.starts_with("#!/usr/bin/env python") || text.starts_with("#!/usr/bin/python") {
        return Some("python");
    }
    if text.starts_with("<!DOCTYPE html>") || text.starts_with("<!doctype html>") {
        return Some("html");
    }
    if text.starts_with("package ") && sample.contains("\nfunc ") {
        return Some("go");
    }
    if (text.starts_with("use ") || text.starts_with("pub ") || text.starts_with("fn "))
        && (sample.contains("fn ") || sample.contains("impl "))
    {
        return Some("rust");
    }
    if text.starts_with("def ") && text.lines().next().is_some_and(|l| l.ends_with(':')) {
        return Some("python");
    }
    if text.starts_with("import ") && sample.contains("\ndef ") {
        return Some("python");
    }
    if text.starts_with("interface ") || text.starts_with("export interface ") {
        return Some("typescript");
    }
    if text.starts_with("const ") && (sample.contains("=>") || sample.contains("require(")) {
        return Some("javascript");
    }
    None
}

fn hunk_start(line: &str) -> Option<(usize, usize, usize, usize)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    fn side(side: &str) -> Option<(usize, usize)> {
        match side.split_once(',') {
            Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
            None => Some((side.parse().ok()?, 1)),
        }
    }
    let (old, old_count) = side(old)?;
    let (new, new_count) = side(new)?;
    Some((old, new, old_count, new_count))
}

impl CodePresentation {
    pub fn new(code: &str, language: Option<&str>) -> Self {
        let language = language
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .or_else(|| inferred_language(code).map(str::to_owned));
        let diff = language
            .as_deref()
            .is_some_and(|l| matches!(l, "diff" | "patch"));
        let mut gutter = String::new();
        let mut changes = Vec::new();
        let mut old_new = None;
        for (ix, line) in code.split_terminator('\n').enumerate() {
            if ix > 0 {
                gutter.push('\n');
            }
            if diff {
                if old_new.is_none()
                    && (line.starts_with("diff --git ")
                        || line.starts_with("--- ")
                        || line.starts_with("+++ "))
                {
                    old_new = None;
                    gutter.push_str("   ·    ·");
                    continue;
                }
                if let Some(start) = hunk_start(line) {
                    old_new = Some(start);
                    gutter.push_str("   ·    ·");
                    continue;
                }
                if let Some((old, new, old_left, new_left)) = old_new.as_mut() {
                    match line.as_bytes().first() {
                        Some(b'+') if *new_left > 0 => {
                            gutter.push_str(&format!("     {new:>4}"));
                            *new = new.saturating_add(1);
                            *new_left = new_left.saturating_sub(1);
                            changes.push((ix, true));
                        }
                        Some(b'-') if *old_left > 0 => {
                            gutter.push_str(&format!("{old:>4}     "));
                            *old = old.saturating_add(1);
                            *old_left = old_left.saturating_sub(1);
                            changes.push((ix, false));
                        }
                        Some(b' ') if *old_left > 0 && *new_left > 0 => {
                            gutter.push_str(&format!("{old:>4} {new:>4}"));
                            *old = old.saturating_add(1);
                            *old_left = old_left.saturating_sub(1);
                            *new = new.saturating_add(1);
                            *new_left = new_left.saturating_sub(1);
                        }
                        Some(b'\\') => gutter.push_str("   ·    ·"),
                        _ => {
                            old_new = None;
                            gutter.push_str("   ·    ·");
                        }
                    }
                    if old_new
                        .as_ref()
                        .is_some_and(|(_, _, old_left, new_left)| *old_left == 0 && *new_left == 0)
                    {
                        old_new = None;
                    }
                } else {
                    gutter.push_str(&(ix + 1).to_string());
                    if line.starts_with('+') {
                        changes.push((ix, true));
                    } else if line.starts_with('-') {
                        changes.push((ix, false));
                    }
                }
            } else {
                gutter.push_str(&(ix + 1).to_string());
            }
        }
        let gutter_columns = gutter
            .lines()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(1);
        Self {
            language: language.map(Into::into),
            gutter: gutter.into(),
            changes,
            gutter_columns,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diff_gutter_tracks_hunks_and_does_not_count_headers() {
        let p = CodePresentation::new(
            "--- a/a\n+++ b/a\n@@ -4,2 +8,2 @@\n same\n-old\n+new\n\\ No newline at end of file\n@@ -20 +30 @@\n-x\n+y\n",
            Some("diff"),
        );
        let rows: Vec<_> = p.gutter.lines().collect();
        assert_eq!(rows[3], "   4    8");
        assert_eq!(rows[4], "   5     ");
        assert_eq!(rows[5], "        9");
        assert_eq!(rows[8], "  20     ");
        assert_eq!(rows[9], "       30");
        assert_eq!(
            p.changes,
            vec![(4, false), (5, true), (8, false), (9, true)]
        );
    }
    #[test]
    fn header_only_diff_and_multiple_files_do_not_color_headers() {
        let p = CodePresentation::new(
            "-old\n+new\ndiff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -99999 +100000 @@\n-old\n+new\n--- a/b\n+++ b/b\n",
            Some("diff"),
        );
        assert_eq!(
            p.changes,
            vec![(0, false), (1, true), (6, false), (7, true)]
        );
        assert!(p.gutter_columns >= 11);
        let code = format!("@@ -{} +{} @@\n x\n-y\n+z", usize::MAX, usize::MAX);
        let p = CodePresentation::new(&code, Some("diff"));
        assert_eq!(p.changes.len(), 2);
    }
    #[test]
    fn hunk_content_that_looks_like_file_headers_keeps_correct_numbers() {
        let p = CodePresentation::new(
            "@@ -7,2 +9,2 @@\n--- old source\n+++ new source\n tail\n--- a/next\n+++ b/next\n@@ -1,0 +2,1 @@\n+added",
            Some("diff"),
        );
        let rows: Vec<_> = p.gutter.lines().collect();
        assert_eq!(rows[1], "   7     ");
        assert_eq!(rows[2], "        9");
        assert_eq!(rows[3], "   8   10");
        assert_eq!(p.changes, vec![(1, false), (2, true), (7, true)]);
        assert_eq!(hunk_start("@@ -1,no +2 @@"), None);
    }
    #[test]
    fn inference_requires_patch_structure_and_respects_explicit_language() {
        assert_eq!(inferred_language("- item\n+ item\n"), None);
        assert_eq!(
            inferred_language("--- old\n+++ new\n@@ -1 +1 @@\n-a\n+b"),
            Some("diff")
        );
        assert_eq!(
            CodePresentation::new("fn main() {}", Some("text"))
                .language
                .as_deref(),
            Some("text")
        );
        assert_eq!(
            CodePresentation::new("fn main() {}", None)
                .language
                .as_deref(),
            Some("rust")
        );
        assert!(inferred_language(&"fn main() {}".repeat(20_000)).is_none());
    }
}
