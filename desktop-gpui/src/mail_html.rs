//! Ephemeral, script-free mail document. No IPC, browser profile, or navigation.
use gpui_kit::{prelude::*, *};
use gpui_wry::WebView;
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

const MAX_BODY: usize = 2 * 1024 * 1024;
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn quoted_text(value: &str) -> String {
    let mut result = String::from("<div class=plain-message>");
    let mut depth = 0;
    for line in value.lines() {
        let mut body = line.trim_start();
        let mut next = 0;
        while let Some(rest) = body.strip_prefix('>') {
            next += 1;
            body = rest.trim_start();
            if next == 32 {
                break;
            }
        }
        if next == 0 {
            body = line;
        }
        while depth < next {
            result.push_str("<blockquote>");
            depth += 1;
        }
        while depth > next {
            result.push_str("</blockquote>");
            depth -= 1;
        }
        result.push_str("<div>");
        if body.is_empty() {
            result.push_str("<br>");
        } else {
            result.push_str(&escape(body));
        }
        result.push_str("</div>");
    }
    result.push_str(&"</blockquote>".repeat(depth));
    result.push_str("</div>");
    result
}
fn sanitized_html(value: &str) -> String {
    let mut builder = ammonia::Builder::new();
    let tags: HashSet<_> = [
        "a",
        "abbr",
        "b",
        "blockquote",
        "br",
        "caption",
        "code",
        "col",
        "colgroup",
        "dd",
        "del",
        "div",
        "dl",
        "dt",
        "em",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "hr",
        "i",
        "img",
        "li",
        "ol",
        "p",
        "pre",
        "s",
        "small",
        "span",
        "strong",
        "sub",
        "sup",
        "table",
        "tbody",
        "td",
        "th",
        "thead",
        "tr",
        "u",
        "ul",
    ]
    .into_iter()
    .collect();
    let attrs: HashMap<_, HashSet<_>> = [
        (
            "img",
            ["src", "alt", "width", "height"].into_iter().collect(),
        ),
        ("td", ["colspan", "rowspan"].into_iter().collect()),
        ("th", ["colspan", "rowspan"].into_iter().collect()),
    ]
    .into_iter()
    .collect();
    builder
        .tags(tags)
        .tag_attributes(attrs)
        .generic_attributes(["class", "title"].into_iter().collect())
        .url_schemes(["data"].into_iter().collect())
        .url_relative(ammonia::UrlRelative::Deny)
        .link_rel(None)
        .attribute_filter(|tag, attribute, value| {
            if tag == "img" && attribute == "src" {
                let lower = value.to_ascii_lowercase();
                if value.len() > MAX_BODY
                    || ![
                        "data:image/png;base64,",
                        "data:image/jpeg;base64,",
                        "data:image/gif;base64,",
                        "data:image/webp;base64,",
                    ]
                    .iter()
                    .any(|prefix| lower.starts_with(prefix))
                {
                    return None;
                }
            }
            if attribute == "class" {
                let classes = value
                    .split_whitespace()
                    .filter(|name| ["gmail_quote", "gmail_attr"].contains(name))
                    .collect::<Vec<_>>()
                    .join(" ");
                return (!classes.is_empty()).then(|| Cow::Owned(classes));
            }
            Some(Cow::Borrowed(value))
        });
    builder.clean(value).to_string()
}
pub(super) fn document(html: Option<&str>, plain: &str, dark: bool) -> Result<String, String> {
    if html.is_some_and(|body| body.len() > MAX_BODY) || plain.len() > MAX_BODY {
        return Err(
            "Mail body exceeds the bounded native reader limit. Plain text remains available."
                .into(),
        );
    }
    let content = html
        .filter(|body| !body.trim().is_empty())
        .map(sanitized_html)
        .unwrap_or_else(|| quoted_text(plain));
    let (background, foreground, muted, line) = if dark {
        ("#202020", "#dedede", "#aaaaaa", "#555555")
    } else {
        ("#ffffff", "#25313b", "#526477", "#b8cbdc")
    };
    Ok(format!(
        "<!doctype html><html><head><meta charset=utf-8><meta http-equiv=Content-Security-Policy content=\"default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; media-src 'none'; form-action 'none'; base-uri 'none'\"><style>:root{{color-scheme:{}}}*{{box-sizing:border-box}}body{{margin:0;padding:24px;color:{foreground};background:{background};font:14px/1.65 -apple-system,sans-serif;overflow-wrap:anywhere}}img,table{{max-width:100%}}img{{height:auto}}pre,.plain-message{{white-space:pre-wrap}}blockquote,.gmail_quote{{margin:14px 0 14px 8px;padding:4px 0 4px 16px;border-left:3px solid {line};color:{muted}}}blockquote blockquote{{border-color:#a18bc4}}blockquote blockquote blockquote{{border-color:#739e86}}.gmail_attr{{font-size:12px;color:{muted}}}a{{color:#5c88b3}}hr{{border:0;border-top:1px solid {line}}}</style></head><body>{content}</body></html>",
        if dark { "dark" } else { "light" }
    ))
}
pub(super) struct MailHtmlReader {
    webview: Option<Entity<WebView>>,
}
impl MailHtmlReader {
    pub fn open(
        document: String,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Entity<Self>, String> {
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (document, window, cx);
            Err("Native rich mail reader currently supports macOS only.".into())
        }
        #[cfg(target_os = "macos")]
        {
            let raw = wry::WebViewBuilder::new()
                .with_incognito(true)
                .with_javascript_disabled()
                .with_html(document)
                .with_visible(false)
                .with_focused(false)
                .with_navigation_handler(|url| url == "about:blank")
                .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
                .build_as_child(&*window)
                .map_err(|error| error.to_string())?;
            let view = cx.new(|cx| WebView::new(raw, window, cx));
            // The parent creates this only for the active, selected detail.
            view.update(cx, |view, _| view.show());
            Ok(cx.new(|_| Self {
                webview: Some(view),
            }))
        }
    }
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = self.webview.take() {
            view.update(cx, |view, _| view.hide());
        }
        cx.notify();
    }
}
impl Render for MailHtmlReader {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut body = div().size_full().overflow_hidden();
        if let Some(view) = &self.webview {
            body = body.child(view.clone());
        }
        body
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn malicious_markup_has_no_execution_navigation_or_remote_resource() {
        let result = sanitized_html(
            "<script>alert(1)</script><iframe src='https://tracker.invalid'></iframe><a href='javascript:bad()' target=_blank>label</a><img src='https://tracker.invalid/pixel' onerror='bad()'><form action='https://tracker.invalid'><input></form><div style='background:url(https://tracker.invalid)' onclick='bad()'>text</div>",
        );
        for forbidden in [
            "<script",
            "<iframe",
            "href=",
            "target=",
            "src=",
            "onerror",
            "onclick",
            "<form",
            "<input",
            "style=",
            "https:",
            "javascript:",
        ] {
            assert!(!result.contains(forbidden), "{forbidden}: {result}");
        }
    }
    #[::core::prelude::v1::test]
    fn inline_images_are_raster_only_and_quote_classes_survive() {
        let result = sanitized_html(
            "<div class='gmail_quote evil'>quoted</div><img src='data:image/svg+xml;base64,PHN2Zz4='><img src='data:image/png;base64,iVBORw0KGgo='>",
        );
        assert!(result.contains("class=\"gmail_quote\""));
        assert!(!result.contains("evil"));
        assert!(!result.contains("svg+xml"));
        assert!(result.contains("data:image/png;base64,"));
    }
    #[::core::prelude::v1::test]
    fn document_enforces_network_and_script_policy_and_size_bound() {
        let result = document(None, "test", true).unwrap();
        assert!(result.contains("default-src 'none'"));
        assert!(result.contains("script-src 'none'"));
        assert!(result.contains("connect-src 'none'"));
        assert!(result.contains("color-scheme:dark"));
        assert!(document(Some(&"x".repeat(MAX_BODY + 1)), "", false).is_err());
    }
    #[::core::prelude::v1::test]
    fn plain_quotes_are_nested_and_escaped() {
        let result = quoted_text("hello\n> quoted\n>> <script>\nback");
        assert!(result.contains("<blockquote><div>quoted</div><blockquote>"));
        assert!(result.contains("&lt;script&gt;"));
        assert_eq!(
            result.matches("<blockquote>").count(),
            result.matches("</blockquote>").count()
        );
    }
}
