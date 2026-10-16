//! URL and search interpretation for the minimal browser address composer.
use url::Url;

pub(super) fn navigation_target(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Enter an address or search terms.".into());
    }
    if text.len() > super::MAX_BROWSER_URL_BYTES {
        return Err("Browser URL exceeds the 8192 byte limit.".into());
    }
    if std::path::Path::new(text).is_absolute() {
        return monitter_lib::browser_links::normalize_target(text);
    }
    // Explicit schemes always go through the browser's strict scheme and
    // credential validation, rather than being silently turned into searches.
    let explicit_scheme = text.split_once(':').is_some_and(|(scheme, _)| {
        !scheme.is_empty()
            && scheme
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))
            && !(scheme == "localhost" || scheme.contains('.'))
    });
    if explicit_scheme {
        return super::validated_url(text).map(|url| url.to_string());
    }
    if !text.chars().any(char::is_whitespace)
        && (text.contains('.') || text.starts_with("localhost") || text.starts_with("[::1]"))
    {
        let local = text == "localhost"
            || text.starts_with("localhost:")
            || text.starts_with("localhost/")
            || text.starts_with("127.")
            || text.starts_with("[::1]");
        let scheme = if local { "http" } else { "https" };
        return super::validated_url(&format!("{scheme}://{text}")).map(|url| url.to_string());
    }
    let mut url = Url::parse("https://www.google.com/search").expect("constant search URL");
    url.query_pairs_mut().append_pair("q", text);
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_addresses_and_encodes_search_without_allowing_privileged_schemes() {
        assert_eq!(
            navigation_target("example.com/path").unwrap(),
            "https://example.com/path"
        );
        assert_eq!(
            navigation_target("localhost:3000").unwrap(),
            "http://localhost:3000/"
        );
        assert_eq!(navigation_target("about:blank").unwrap(), "about:blank");
        let search = Url::parse(&navigation_target("rust & gpui").unwrap()).unwrap();
        assert_eq!(search.query_pairs().next().unwrap().1, "rust & gpui");
        for input in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,hi",
            "https://user:password@example.com",
            "",
        ] {
            assert!(navigation_target(input).is_err(), "{input}");
        }
    }
    #[test]
    fn handles_loopback_and_preserves_explicit_https() {
        assert_eq!(
            navigation_target("127.0.0.1:8080/test").unwrap(),
            "http://127.0.0.1:8080/test"
        );
        assert_eq!(
            navigation_target("[::1]:3000").unwrap(),
            "http://[::1]:3000/"
        );
        assert_eq!(
            navigation_target("https://localhost:3000").unwrap(),
            "https://localhost:3000/"
        );
        assert!(navigation_target(&"x".repeat(super::super::MAX_BROWSER_URL_BYTES + 1)).is_err());
    }
}
