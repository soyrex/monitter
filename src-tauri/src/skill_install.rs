//! Downloads portable Markdown skill instructions without installing or running
//! any bundled code.  This module intentionally has no filesystem side effects.

use std::{
    io::Read,
    net::{IpAddr, Ipv4Addr, ToSocketAddrs},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

const MAX_SKILL_BYTES: usize = 128 * 1024;
const FETCH_TIMEOUT_SECS: u64 = 15;
const DNS_TIMEOUT_SECS: u64 = 5;
const STATUS_MARKER: &[u8] = b"\n__MONITTER_HTTP_STATUS__:";
const STATUS_TRAILER_BYTES: usize = STATUS_MARKER.len() + 4;

pub(crate) struct DownloadedSkill {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) content: String,
    pub(crate) source_url: String,
}

struct FetchTarget {
    url: String,
    host: String,
    port: u16,
}

/// Fetch a portable Markdown skill. The text is returned for review/import by
/// the caller; downloaded instructions, scripts, and assets are never executed.
pub(crate) fn download_skill(url: &str) -> Result<DownloadedSkill, String> {
    let target = normalize_url(url)?;
    let bytes = fetch_https(&target)?;
    let content = String::from_utf8(bytes)
        .map_err(|_| "Downloaded skill is not UTF-8 Markdown.".to_string())?;
    validate_content(&content)?;
    let (name, description) = metadata(&content, &target.url);
    Ok(DownloadedSkill {
        name,
        description,
        content,
        source_url: target.url,
    })
}

fn normalize_url(input: &str) -> Result<FetchTarget, String> {
    let input = input.trim();
    if input.is_empty() || input.len() > 8 * 1024 || input.contains(['\r', '\n', '\0']) {
        return Err("Enter one HTTPS Markdown URL.".into());
    }
    let (host, _port, path) = parse_https(input)?;
    let url = if host.eq_ignore_ascii_case("github.com") {
        github_raw_url(&path)?
    } else {
        if input.contains('?') {
            return Err(
                "Use a public credential-free Markdown URL without query parameters.".into(),
            );
        }
        if !markdown_path(&path) {
            return Err("The URL must point directly to a Markdown file or SKILL.md.".into());
        }
        input.to_string()
    };
    let (host, port, path) = parse_https(&url)?;
    if !markdown_path(&path) {
        return Err("The URL did not resolve to a Markdown skill file.".into());
    }
    Ok(FetchTarget { url, host, port })
}

fn parse_https(url: &str) -> Result<(String, u16, String), String> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| "Only HTTPS skill URLs are allowed.".to_string())?;
    if rest.contains('@') {
        return Err("Skill URLs cannot contain credentials.".into());
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() {
        return Err("Skill URL has no host.".into());
    }
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let end = bracketed
            .find(']')
            .ok_or_else(|| "Invalid IPv6 skill URL.".to_string())?;
        let host = format!("[{}]", &bracketed[..end]);
        let suffix = &bracketed[end + 1..];
        let port = suffix
            .strip_prefix(':')
            .map(parse_port)
            .transpose()?
            .unwrap_or(443);
        if !suffix.is_empty() && !suffix.starts_with(':') {
            return Err("Invalid skill URL host.".into());
        }
        (host, port)
    } else if let Some((host, value)) = authority.rsplit_once(':') {
        if host.contains(':') {
            return Err("Invalid skill URL host.".into());
        }
        (host.to_string(), parse_port(value)?)
    } else {
        (authority.to_string(), 443)
    };
    if port != 443 {
        return Err("Skill URLs must use HTTPS port 443.".into());
    }
    if host.is_empty()
        || host
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
            && !host.starts_with('[')
    {
        return Err("Invalid skill URL host.".into());
    }
    let path = &rest[authority_end..];
    let path = path.split(['?', '#']).next().unwrap_or("");
    Ok((host.to_ascii_lowercase(), port, path.to_string()))
}

fn parse_port(value: &str) -> Result<u16, String> {
    value
        .parse::<u16>()
        .map_err(|_| "Invalid skill URL port.".to_string())
}

fn github_raw_url(path: &str) -> Result<String, String> {
    let segments: Vec<_> = path
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    if segments.len() < 2 {
        return Err("A GitHub skill URL needs an owner and repository.".into());
    }
    let owner = segments[0];
    let repo = segments[1];
    if !github_part(owner) || !github_part(repo) {
        return Err("Invalid GitHub repository URL.".into());
    }
    let raw_path = match segments.get(2).copied() {
        None => "HEAD/SKILL.md".to_string(),
        Some("blob") if segments.len() >= 5 => segments[3..].join("/"),
        Some("tree") if segments.len() >= 4 => {
            let mut result = segments[3..].join("/");
            if !result.ends_with('/') {
                result.push('/');
            }
            result.push_str("SKILL.md");
            result
        }
        _ => return Err("Use a GitHub repository, blob, or tree URL for a skill.".into()),
    };
    if !markdown_path(&raw_path) {
        return Err("The GitHub URL does not identify a Markdown skill.".into());
    }
    Ok(format!(
        "https://raw.githubusercontent.com/{owner}/{repo}/{raw_path}"
    ))
}

fn github_part(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

fn markdown_path(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".md") || path.to_ascii_lowercase().ends_with(".mdx")
}

fn fetch_https(target: &FetchTarget) -> Result<Vec<u8>, String> {
    let addresses = resolve_public_addresses(&target.host, target.port)?;
    let mut command = Command::new("curl");
    command.args([
        "-q",
        "--fail",
        "--silent",
        "--show-error",
        "--globoff",
        "--noproxy",
        "*",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        "5",
        "--max-redirs",
        "0",
        "--header",
        "Accept: text/markdown, text/plain;q=0.9",
    ]);
    command
        .arg("--max-time")
        .arg(FETCH_TIMEOUT_SECS.to_string());
    command
        .arg("--max-filesize")
        .arg(MAX_SKILL_BYTES.to_string());
    let addresses = addresses
        .into_iter()
        .map(curl_address)
        .collect::<Vec<_>>()
        .join(",");
    command
        .arg("--resolve")
        .arg(format!("{}:{}:{addresses}", target.host, target.port));
    let mut child = command
        .arg("--write-out")
        .arg("\n__MONITTER_HTTP_STATUS__:%{http_code}\n")
        .arg(&target.url)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Cannot start the HTTPS skill downloader.".to_string())?;
    let max_response_bytes = MAX_SKILL_BYTES + STATUS_TRAILER_BYTES;
    let mut output = Vec::with_capacity(max_response_bytes);
    let read_result = child
        .stdout
        .take()
        .expect("piped stdout")
        .take((max_response_bytes + 1) as u64)
        .read_to_end(&mut output);
    if read_result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Cannot read the downloaded skill.".into());
    }
    if output.len() > max_response_bytes {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Downloaded skill exceeds the 128 KiB limit.".into());
    }
    let status = child
        .wait()
        .map_err(|_| "Cannot complete the HTTPS skill download.".to_string())?;
    if !status.success() {
        return Err("Skill download failed; check that the URL is public HTTPS Markdown (redirects are not followed).".into());
    }
    let (content, http_status) = split_status_trailer(&output)?;
    if (300..400).contains(&http_status) {
        return Err("Skill URL redirected; redirects are not followed for safety. Use the final HTTPS Markdown URL.".into());
    }
    if http_status != 200 {
        return Err(format!(
            "Skill download returned HTTP status {http_status}."
        ));
    }
    if content.len() > MAX_SKILL_BYTES {
        return Err("Downloaded skill exceeds the 128 KiB limit.".into());
    }
    Ok(content.to_vec())
}

fn curl_address(address: IpAddr) -> String {
    match address {
        IpAddr::V4(address) => address.to_string(),
        IpAddr::V6(address) => format!("[{address}]"),
    }
}

fn split_status_trailer(output: &[u8]) -> Result<(&[u8], u16), String> {
    let Some(marker) = output
        .windows(STATUS_MARKER.len())
        .rposition(|part| part == STATUS_MARKER)
    else {
        return Err("Skill downloader did not return an HTTP status.".into());
    };
    let trailer = &output[marker + STATUS_MARKER.len()..];
    if trailer.len() != 4 || trailer[3] != b'\n' || !trailer[..3].iter().all(u8::is_ascii_digit) {
        return Err("Skill downloader returned an invalid HTTP status.".into());
    }
    let status = std::str::from_utf8(&trailer[..3])
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| "Skill downloader returned an invalid HTTP status.".to_string())?;
    Ok((&output[..marker], status))
}

fn resolve_public_addresses(host: &str, port: u16) -> Result<Vec<IpAddr>, String> {
    let bare_host = host.trim_matches(['[', ']']);
    let host = bare_host.to_string();
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let result = (host.as_str(), port)
            .to_socket_addrs()
            .map(|addresses| addresses.map(|address| address.ip()).collect::<Vec<_>>());
        let _ = sender.send(result);
    });
    let resolved = receiver
        .recv_timeout(Duration::from_secs(DNS_TIMEOUT_SECS))
        .map_err(|_| "Skill URL DNS lookup timed out.".to_string())?
        .map_err(|_| "Cannot resolve the skill URL host.".to_string())?;
    let addresses: Vec<IpAddr> = resolved
        .into_iter()
        .filter(|ip| !is_private_ip(*ip))
        .collect();
    if addresses.is_empty() {
        return Err("Skill URL resolves only to a private, loopback, or reserved address.".into());
    }
    Ok(addresses)
}

fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_unspecified()
                || ip.is_documentation()
                || ip.octets()[0] == 0
                || ip.octets()[0] >= 224
                || (ip.octets()[0] == 100 && (ip.octets()[1] & 0xc0) == 0x40) // 100.64.0.0/10 carrier-grade NAT
                || (ip.octets()[0] == 192 && ip.octets()[1] == 0) // 192.0.0.0/24 special use
                || (ip.octets()[0] == 192 && ip.octets()[1] == 88 && ip.octets()[2] == 99) // deprecated IPv6 relay
                || (ip.octets()[0] == 198 && (ip.octets()[1] == 18 || ip.octets()[1] == 19))
            // benchmarking
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_private_ip(IpAddr::V4(mapped));
            }
            let segments = ip.segments();
            if segments[..6].iter().all(|segment| *segment == 0) {
                let compatible = Ipv4Addr::new(
                    (segments[6] >> 8) as u8,
                    segments[6] as u8,
                    (segments[7] >> 8) as u8,
                    segments[7] as u8,
                );
                return is_private_ip(IpAddr::V4(compatible));
            }
            ip.is_loopback()
                || ip.is_unspecified()
                || (segments[0] & 0xfe00) == 0xfc00 // fc00::/7 unique local
                || (segments[0] & 0xffc0) == 0xfe80 // fe80::/10 link local
                || (segments[0] == 0x2001 && segments[1] == 0x0db8) // documentation
                || ip.is_multicast()
                // NAT64 translation prefixes can target an embedded private IPv4
                // address through a local translator. They are not needed for this
                // public skill importer, so reject them as a class.
                || (segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2..6].iter().all(|segment| *segment == 0))
                || (segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2] == 1)
                || (segments[0] == 0x2001 && segments[1] == 0) // Teredo
                || (segments[0] == 0x2002) // 6to4
                || (segments[4] == 0 && segments[5] == 0x5efe
                    && is_private_ip(IpAddr::V4(Ipv4Addr::new(
                        (segments[6] >> 8) as u8,
                        segments[6] as u8,
                        (segments[7] >> 8) as u8,
                        segments[7] as u8,
                    )))) // ISATAP with a private embedded IPv4
        }
    }
}

fn validate_content(content: &str) -> Result<(), String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("Downloaded skill is empty.".into());
    }
    let lower = trimmed
        .chars()
        .take(1024)
        .collect::<String>()
        .to_ascii_lowercase();
    if lower.starts_with("<!doctype html") || lower.starts_with("<html") || lower.contains("<head>")
    {
        return Err("Downloaded page is HTML, not a portable Markdown skill.".into());
    }
    if is_shell_installer(trimmed) {
        return Err(
            "Downloaded page appears to be a shell installer, not portable Markdown instructions."
                .into(),
        );
    }
    Ok(())
}

fn is_shell_installer(content: &str) -> bool {
    let lower = content
        .chars()
        .take(2048)
        .collect::<String>()
        .to_ascii_lowercase();
    (lower.starts_with("#!/bin/")
        && (lower.contains("curl ") || lower.contains("wget ") || lower.contains(" install ")))
        || ((lower.starts_with("curl ") || lower.starts_with("wget "))
            && (lower.contains("| sh") || lower.contains("| bash")))
}

fn metadata(content: &str, source_url: &str) -> (String, String) {
    let mut name = None;
    let mut description = None;
    if let Some(frontmatter) = content
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---").map(|(head, _)| head))
    {
        for line in frontmatter.lines() {
            if let Some(value) = line.strip_prefix("name:") {
                name = clean_metadata(value);
            }
            if let Some(value) = line.strip_prefix("description:") {
                description = clean_metadata(value);
            }
        }
    }
    (
        name.unwrap_or_else(|| filename_fallback(source_url)),
        description.unwrap_or_default(),
    )
}

fn clean_metadata(value: &str) -> Option<String> {
    let value = value.trim().trim_matches(['\'', '"']).trim();
    (!value.is_empty()).then(|| value.chars().take(128).collect())
}

fn filename_fallback(source_url: &str) -> String {
    let path = source_url.split(['?', '#']).next().unwrap_or(source_url);
    let file = path.rsplit('/').next().unwrap_or("skill");
    let stem = file
        .strip_suffix(".md")
        .or_else(|| file.strip_suffix(".MD"))
        .unwrap_or(file);
    if stem.eq_ignore_ascii_case("skill") {
        "skill".into()
    } else {
        stem.chars().take(128).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_github_repo_blob_and_tree_urls() {
        assert_eq!(
            normalize_url("https://github.com/acme/demo").unwrap().url,
            "https://raw.githubusercontent.com/acme/demo/HEAD/SKILL.md"
        );
        assert_eq!(
            normalize_url("https://github.com/acme/demo/blob/main/docs/SKILL.md")
                .unwrap()
                .url,
            "https://raw.githubusercontent.com/acme/demo/main/docs/SKILL.md"
        );
        assert_eq!(
            normalize_url("https://github.com/acme/demo/tree/main/docs")
                .unwrap()
                .url,
            "https://raw.githubusercontent.com/acme/demo/main/docs/SKILL.md"
        );
        assert_eq!(
            normalize_url("https://github.com/acme/demo?tab=readme")
                .unwrap()
                .url,
            "https://raw.githubusercontent.com/acme/demo/HEAD/SKILL.md"
        );
    }

    #[test]
    fn rejects_non_https_credentials_and_non_markdown_urls() {
        assert!(normalize_url("http://example.com/SKILL.md").is_err());
        assert!(normalize_url("https://person@example.com/SKILL.md").is_err());
        assert!(normalize_url("https://example.com/install.sh").is_err());
        assert!(normalize_url("https://example.com:444/SKILL.md").is_err());
        assert!(normalize_url("https://example.com/SKILL.md?token=secret").is_err());
    }

    #[test]
    fn rejects_private_and_reserved_ips() {
        assert!(is_private_ip("127.0.0.1".parse().unwrap()));
        assert!(is_private_ip("10.0.0.1".parse().unwrap()));
        assert!(is_private_ip("169.254.1.1".parse().unwrap()));
        assert!(is_private_ip("100.64.0.1".parse().unwrap()));
        assert!(is_private_ip("198.18.0.1".parse().unwrap()));
        assert!(is_private_ip("::1".parse().unwrap()));
        assert!(is_private_ip("::ffff:127.0.0.1".parse().unwrap()));
        assert!(is_private_ip("::127.0.0.1".parse().unwrap()));
        assert!(is_private_ip("64:ff9b::7f00:1".parse().unwrap()));
        assert!(is_private_ip("2002:7f00:1::1".parse().unwrap()));
        assert!(!is_private_ip("2001:4860:4860::8888".parse().unwrap()));
        assert!(!is_private_ip("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn extracts_frontmatter_and_rejects_non_skill_pages() {
        let text = "---\nname: Useful skill\ndescription: Does useful things\n---\n# Steps";
        assert_eq!(
            metadata(text, "https://example.com/SKILL.md"),
            ("Useful skill".into(), "Does useful things".into())
        );
        assert!(validate_content("<!doctype html><html><head></head></html>").is_err());
        assert!(validate_content("#!/bin/sh\ncurl https://x | sh").is_err());
        assert!(validate_content("# Safe instructions\nRead this first.").is_ok());
    }

    #[test]
    fn parses_status_without_counting_trailer_or_panicking_on_unicode() {
        let response =
            b"# Skill\n__MONITTER_HTTP_STATUS__:not-this-one\n__MONITTER_HTTP_STATUS__:302\n";
        let (body, status) = split_status_trailer(response).unwrap();
        assert_eq!(body, b"# Skill\n__MONITTER_HTTP_STATUS__:not-this-one");
        assert_eq!(status, 302);
        assert!(validate_content(&format!("{}<html", "\u{00e9}".repeat(1024))).is_ok());
        assert_eq!(
            curl_address("2001:4860:4860::8888".parse().unwrap()),
            "[2001:4860:4860::8888]"
        );
        let target = normalize_url("https://[2001:4860:4860::8888]/SKILL.md").unwrap();
        assert_eq!(target.host, "[2001:4860:4860::8888]");
        assert_eq!(
            format!(
                "{}:{}:{}",
                target.host,
                target.port,
                curl_address("2001:4860:4860::8888".parse().unwrap())
            ),
            "[2001:4860:4860::8888]:443:[2001:4860:4860::8888]"
        );
    }
}
