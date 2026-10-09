//! Local wallpaper discovery for worker threads. No screen capture or daemon.
//! Windows exposes one system wallpaper; Linux supports GNOME/Unity, Cinnamon,
//! and MATE. Other desktops, slideshows/XML backgrounds, and per-monitor choices
//! intentionally return None so the renderer can use its normal fallback.
use std::path::PathBuf;

#[cfg(target_os = "windows")]
pub fn resolve() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SystemParametersInfoW(
            action: u32,
            size: u32,
            value: *mut std::ffi::c_void,
            flags: u32,
        ) -> i32;
    }
    let mut buffer = [0_u16; 32768];
    // SPI_GETDESKWALLPAPER writes a NUL-terminated path into the supplied buffer.
    let ok = unsafe {
        SystemParametersInfoW(0x0073, buffer.len() as u32, buffer.as_mut_ptr().cast(), 0)
    };
    if ok == 0 {
        return None;
    }
    let length = buffer.iter().position(|c| *c == 0)?;
    let path = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length]));
    local_file(path)
}

#[cfg(target_os = "linux")]
pub fn resolve() -> Option<PathBuf> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok()?;
    let kind = desktop
        .split(':')
        .find_map(|part| match part.to_ascii_lowercase().as_str() {
            "gnome" | "unity" => Some("gnome"),
            "x-cinnamon" | "cinnamon" => Some("cinnamon"),
            "mate" => Some("mate"),
            _ => None,
        })?;
    let (schema, key) = match kind {
        "gnome" => {
            let dark = setting("org.gnome.desktop.interface", "color-scheme")
                .as_deref()
                .is_some_and(|s| s == "prefer-dark");
            if dark {
                if let Some(path) = setting("org.gnome.desktop.background", "picture-uri-dark")
                    .and_then(|s| parse_path(&s))
                {
                    return Some(path);
                }
            }
            ("org.gnome.desktop.background", "picture-uri")
        }
        "cinnamon" => ("org.cinnamon.desktop.background", "picture-uri"),
        _ => ("org.mate.background", "picture-filename"),
    };
    setting(schema, key).and_then(|s| parse_path(&s))
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn resolve() -> Option<PathBuf> {
    None
}

fn local_file(path: PathBuf) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    if matches!(path.components().next(), Some(std::path::Component::Prefix(prefix))
        if matches!(prefix.kind(), std::path::Prefix::UNC(..) | std::path::Prefix::VerbatimUNC(..)))
    {
        return None;
    }
    if !path.is_absolute() || !path.is_file() {
        return None;
    }
    // GNOME can point at a timed XML slideshow rather than an image.
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
    {
        return None;
    }
    Some(path)
}

#[cfg(any(target_os = "linux", test))]
fn parse_path(value: &str) -> Option<PathBuf> {
    if value.is_empty() || value.contains('\0') {
        return None;
    }
    let path = if value.starts_with("file:") {
        let url = url::Url::parse(value).ok()?;
        if url
            .host_str()
            .is_some_and(|host| !host.is_empty() && host != "localhost")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return None;
        }
        url.to_file_path().ok()?
    } else {
        if value.contains("://") {
            return None;
        }
        PathBuf::from(value)
    };
    local_file(path)
}

/// Decode the string subset of GVariant emitted by gsettings. Never evaluate
/// shell syntax; unfamiliar escape forms fail closed.
#[cfg(any(target_os = "linux", test))]
fn parse_setting(output: &str) -> Option<String> {
    let input = output.trim();
    let quote = input.chars().next()?;
    if !matches!(quote, '\'' | '"') || !input.ends_with(quote) || input.len() < 2 {
        return None;
    }
    let mut chars = input[1..input.len() - 1].chars();
    let mut result = String::new();
    while let Some(c) = chars.next() {
        if c == '\\' {
            result.push(match chars.next()? {
                '\\' => '\\',
                '\'' => '\'',
                '"' => '"',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                _ => return None,
            });
        } else if c == quote {
            return None;
        } else {
            result.push(c);
        }
    }
    Some(result)
}

#[cfg(target_os = "linux")]
fn setting(schema: &str, key: &str) -> Option<String> {
    let output = bounded_command(
        "gsettings",
        &["get", schema, key],
        std::time::Duration::from_millis(500),
    )?;
    parse_setting(&output)
}

#[cfg(any(target_os = "linux", all(test, unix)))]
fn bounded_command(program: &str, args: &[&str], timeout: std::time::Duration) -> Option<String> {
    use std::{
        io::Read,
        os::fd::AsRawFd,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    unsafe extern "C" {
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    }
    #[cfg(target_os = "linux")]
    const NONBLOCK: i32 = 2048;
    #[cfg(not(target_os = "linux"))]
    const NONBLOCK: i32 = 4;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let result = (|| {
        let mut pipe = child.stdout.take()?;
        let flags = unsafe { fcntl(pipe.as_raw_fd(), 3) };
        if flags < 0 || unsafe { fcntl(pipe.as_raw_fd(), 4, flags | NONBLOCK) } < 0 {
            return None;
        }
        let deadline = Instant::now() + timeout;
        let mut output = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            if Instant::now() >= deadline {
                return None;
            }
            match pipe.read(&mut buffer) {
                Ok(0) => {
                    if let Some(status) = child.try_wait().ok()? {
                        return status
                            .success()
                            .then(|| String::from_utf8(output).ok())
                            .flatten();
                    }
                }
                Ok(count) => {
                    if output.len() + count > 16384 {
                        return None;
                    }
                    output.extend_from_slice(&buffer[..count]);
                    continue;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return None,
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    // Reap on every exit, including timeout/output overflow. No reader thread
    // can remain blocked on an inherited pipe after the direct child exits.
    let _ = child.kill();
    let _ = child.wait();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn variant_strings_decode_without_shell_evaluation() {
        assert_eq!(
            parse_setting("'file:///tmp/a%20b.png'\n"),
            Some("file:///tmp/a%20b.png".into())
        );
        assert_eq!(parse_setting("'a\\'b\\\\c'"), Some("a'b\\c".into()));
        assert_eq!(
            parse_setting("'$(touch /tmp/no)'"),
            Some("$(touch /tmp/no)".into())
        );
        assert_eq!(parse_setting("'bad\\q'"), None);
        assert_eq!(parse_setting("'a' 'b'"), None);
    }
    #[test]
    fn only_existing_local_files_are_accepted() {
        let path =
            std::env::temp_dir().join(format!("monitter-platform-{} a.png", std::process::id()));
        std::fs::write(&path, b"test").unwrap();
        assert_eq!(
            parse_path(url::Url::from_file_path(&path).unwrap().as_str()),
            Some(path.clone())
        );
        assert_eq!(parse_path("file://remote/tmp/wallpaper.png"), None);
        assert_eq!(parse_path("https://example.org/a.png"), None);
        assert_eq!(parse_path("relative.png"), None);
        assert_eq!(parse_path(""), None);
        assert_eq!(parse_path(std::env::temp_dir().to_str().unwrap()), None);
        std::fs::remove_file(path).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn subprocess_timeout_output_limit_and_failure_are_bounded() {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        assert_eq!(
            bounded_command(
                "/bin/sh",
                &["-c", "while :; do :; done"],
                Duration::from_millis(50)
            ),
            None
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        assert_eq!(
            bounded_command(
                "/bin/sh",
                &["-c", "while :; do printf abcdefghijklmnopqrstuvwxyz; done"],
                Duration::from_secs(1)
            ),
            None
        );
        assert_eq!(
            bounded_command("/bin/sh", &["-c", "exit 1"], Duration::from_secs(1)),
            None
        );
        assert_eq!(
            bounded_command("/bin/echo", &["hello"], Duration::from_secs(1)),
            Some("hello\n".into())
        );
    }
}
