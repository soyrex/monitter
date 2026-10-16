//! Read-only report presentation snapshots published by the owner-local native GUI.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const RESOURCE_URI: &str = "monitter://reports/theme.css";
pub const USAGE: &str = "Before generating an HTML report, request get_report_styles (or read monitter://reports/theme.css). Embed the returned CSS in a style element, put class=\"monitter-report\" on the body or report root, and retain the responsive viewport meta tag. Use semantic sections, headings, tables and code blocks; available component classes are documented in the CSS. For optional contents navigation, wrap an aside.report-sidebar containing nav.report-toc and main.report-container as siblings in div.report-layout: fixed sidebar on wide screens, wrapping pills on narrow screens, hidden sidebar in print. Standalone nav.report-toc stays a wrapping pill set. Save the HTML file and use browser_open with a stable request_id to show it in Monitter. Styles are a captured snapshot: saved reports do not automatically follow later theme changes. Fonts use local fallbacks and require no external network.";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportTheme {
    pub dark: bool,
    pub accent: u32,
    pub paper: u32,
    pub sidebar: u32,
    pub panel: u32,
    pub line: u32,
    pub soft: u32,
    pub code: u32,
    pub ink: u32,
    pub muted: u32,
    pub font_family: String,
    pub mono_font_family: String,
    pub font_size: f32,
}
impl ReportTheme {
    pub fn validate(&self) -> Result<(), String> {
        if [
            self.accent,
            self.paper,
            self.sidebar,
            self.panel,
            self.line,
            self.soft,
            self.code,
            self.ink,
            self.muted,
        ]
        .iter()
        .any(|color| *color > 0xffffff)
        {
            return Err("Report theme colours must be RGB values.".into());
        }
        for family in [&self.font_family, &self.mono_font_family] {
            if family.trim().is_empty()
                || family.len() > 256
                || family.chars().any(char::is_control)
            {
                return Err(
                    "Report font names must contain 1–256 bytes without control characters.".into(),
                );
            }
        }
        if !self.font_size.is_finite() || !(6. ..=64.).contains(&self.font_size) {
            return Err("Report font size must be finite and between 6 and 64 pixels.".into());
        }
        Ok(())
    }
}
#[derive(Clone)]
pub(crate) struct CapturedTheme {
    pub theme: ReportTheme,
    pub captured_at: i64,
}
fn css_string(value: &str) -> String {
    let mut escaped = String::from("\"");
    for character in value.chars() {
        // Hex escapes also protect the enclosing HTML style element.
        if matches!(character, '"' | '\\' | '<' | '>' | '&') {
            escaped.push_str(&format!("\\{:x} ", character as u32));
        } else {
            escaped.push(character);
        }
    }
    escaped.push('"');
    escaped
}
pub(crate) fn styles(snapshot: &CapturedTheme) -> Result<Value, String> {
    let t = &snapshot.theme;
    t.validate()?;
    let mut css = format!("/* Monitter report styles v1; captured at {} */\n.monitter-report {{\n  color-scheme: {};\n", snapshot.captured_at, if t.dark { "dark" } else { "light" });
    for (name, color) in [
        ("accent", t.accent),
        ("paper", t.paper),
        ("sidebar", t.sidebar),
        ("panel", t.panel),
        ("line", t.line),
        ("soft", t.soft),
        ("code", t.code),
        ("ink", t.ink),
        ("muted", t.muted),
    ] {
        css.push_str(&format!("  --report-{name}: #{color:06x};\n"));
    }
    css.push_str(&format!("  --report-font-family: {}, -apple-system, BlinkMacSystemFont, \"Segoe UI\", sans-serif;\n  --report-mono-font-family: {}, ui-monospace, SFMono-Regular, Menlo, monospace;\n  --report-font-size: {}px;\n}}\n", css_string(&t.font_family), css_string(&t.mono_font_family), t.font_size));
    css.push_str(include_str!("report.css"));
    let starter = format!("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>Report</title>\n<style>\n{css}\n</style>\n</head>\n<body class=\"monitter-report\">\n<main class=\"report-container\">\n<header class=\"report-header\"><p class=\"report-eyebrow\">Report</p><h1>Report title</h1><p class=\"report-lead\">A concise overview.</p></header>\n<section aria-labelledby=\"findings\"><h2 id=\"findings\">Findings</h2><p>Add the report content here.</p></section>\n<footer class=\"report-footer\"><p>Styles captured from Monitter; saved reports retain this appearance.</p></footer>\n</main>\n</body>\n</html>");
    Ok(
        json!({"version":1,"capturedAt":snapshot.captured_at,"theme":t,"css":css,"starterHtml":starter,"usage":USAGE}),
    )
}
impl crate::Service {
    pub(crate) fn publish_report_theme(&self, theme: &ReportTheme) -> Result<(), String> {
        theme.validate()?;
        *self
            .report_theme
            .lock()
            .map_err(|_| "Report theme state unavailable.")? = Some(CapturedTheme {
            theme: theme.clone(),
            captured_at: crate::model::now(),
        });
        Ok(())
    }
    pub(crate) fn get_report_styles(&self) -> Result<Value, String> {
        let snapshot = self.report_theme.lock().map_err(|_| "Report theme state unavailable.")?
            .clone().ok_or_else(|| "Report styles unavailable: the native Monitter GUI has not published its current theme. Open the GUI and retry.".to_string())?;
        styles(&snapshot)
    }
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn theme() -> ReportTheme {
        ReportTheme {
            dark: true,
            accent: 0x38bfd2,
            paper: 0x121212,
            sidebar: 0x181818,
            panel: 0x202020,
            line: 0x343434,
            soft: 0x282828,
            code: 0x101010,
            ink: 0xffffff,
            muted: 0xaaaaaa,
            font_family: "Inter".into(),
            mono_font_family: "Menlo".into(),
            font_size: 14.,
        }
    }
    #[test]
    fn report_styles_validate_boundaries() {
        let mut t = theme();
        assert!(t.validate().is_ok());
        t.accent = 0x1000000;
        assert!(t.validate().is_err());
        t = theme();
        for size in [6.4, 64.] {
            t.font_size = size;
            assert!(t.validate().is_ok());
        }
        for size in [f32::NAN, f32::INFINITY, 0., 5.9, 65.] {
            t.font_size = size;
            assert!(t.validate().is_err());
        }
        t = theme();
        for family in ["".to_string(), "x".repeat(257), "Inter\n".into()] {
            t.font_family = family;
            assert!(t.validate().is_err());
        }
    }
    #[test]
    fn report_styles_escape_fonts_and_produce_embeddable_responsive_snapshot() {
        let mut t = theme();
        t.font_family = "A\";}</style><script>alert(1)</script>\\".into();
        let result = styles(&CapturedTheme {
            theme: t,
            captured_at: 123,
        })
        .unwrap();
        let css = result["css"].as_str().unwrap();
        assert!(css.contains("--report-accent: #38bfd2"));
        assert!(css.contains("\\3c "));
        assert!(!css.contains("</style>"));
        assert!(!css.contains("http://"));
        assert!(!css.contains("https://"));
        let html = result["starterHtml"].as_str().unwrap();
        assert!(html.contains("width=device-width"));
        assert!(html.contains("class=\"monitter-report\""));
        assert_eq!(html.matches("</style>").count(), 1);
        assert_eq!(result["capturedAt"], 123);
    }
}
