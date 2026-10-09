//! Resolve the cached thumbnail of the single movie currently open by Apple's
//! aerial wallpaper provider. Call on a worker and only when aerials is the
//! selected provider: an idle provider process may retain an old open movie.
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
pub fn aerial_thumbnail() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if !home.is_absolute() {
        return None;
    }
    let current = home.join("Library/Application Support/com.apple.wallpaper/aerials");
    let legacy = PathBuf::from("/Library/Application Support/com.apple.idleassetsd/Customer");
    let output = bounded_lsof(std::time::Duration::from_millis(900))?;
    select_thumbnail(&output, &current, &legacy)
}

#[cfg(not(target_os = "macos"))]
pub fn aerial_thumbnail() -> Option<PathBuf> {
    None
}

#[cfg(any(target_os = "macos", test))]
fn asset_id(path: &Path) -> Option<&str> {
    if path.extension()?.to_str()? != "mov" {
        return None;
    }
    let id = path.file_stem()?.to_str()?;
    if id.len() != 36
        || !id.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
    {
        return None;
    }
    Some(id)
}

#[cfg(any(target_os = "macos", test))]
fn select_thumbnail(output: &str, current: &Path, legacy: &Path) -> Option<PathBuf> {
    let current_videos = current.join("videos");
    let mut active: Option<PathBuf> = None;
    let mut thumbnail: Option<PathBuf> = None;
    for line in output.lines() {
        let Some(name) = line.strip_prefix('n') else {
            continue;
        };
        let path = Path::new(name);
        let Some(id) = asset_id(path) else {
            continue;
        };
        let root = if path.parent() == Some(current_videos.as_path()) {
            current
        } else if path.starts_with(legacy) {
            legacy
        } else {
            continue;
        };
        // Canonical checks reject traversal and symlinks escaping the cache.
        let canonical_root = root.canonicalize().ok()?;
        let movie = path.canonicalize().ok()?;
        if !movie.starts_with(&canonical_root) || !movie.is_file() {
            return None;
        }
        if active.as_ref().is_some_and(|previous| previous != &movie) {
            return None; // Multiple displays/assets: no guess or arbitrary first.
        }
        active = Some(movie);
        let candidate = root.join("thumbnails").join(format!("{id}.png"));
        let canonical = candidate.canonicalize().ok()?;
        if !canonical.starts_with(&canonical_root) || !canonical.is_file() {
            return None;
        }
        thumbnail = Some(canonical);
    }
    active.and(thumbnail)
}

#[cfg(target_os = "macos")]
fn bounded_lsof(timeout: std::time::Duration) -> Option<String> {
    use std::{
        io::Read,
        os::fd::AsRawFd,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    unsafe extern "C" {
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    }
    let mut child = Command::new("/usr/sbin/lsof")
        .args(["-a", "-c", "WallpaperAerials", "-d", "0-256", "-Fn"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let result = (|| {
        let mut pipe = child.stdout.take()?;
        let flags = unsafe { fcntl(pipe.as_raw_fd(), 3) };
        if flags < 0 || unsafe { fcntl(pipe.as_raw_fd(), 4, flags | 4) } < 0 {
            return None;
        }
        let deadline = Instant::now() + timeout;
        let mut output = Vec::new();
        let mut buffer = [0_u8; 4096];
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
                    if output.len() + count > 65536 {
                        return None;
                    }
                    output.extend_from_slice(&buffer[..count]);
                    continue;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return None,
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIRST: &str = "3BA0CFC7-E460-4B59-A817-B97F9EBB9B89";
    const SECOND: &str = "5EF2D0B6-0ACB-411A-B92B-90000AAAAAAA";
    fn fixture() -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "monitter-aerial-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let current = base.join("aerials");
        let legacy = base.join("Customer");
        for root in [&current, &legacy] {
            std::fs::create_dir_all(root.join("videos")).unwrap();
            std::fs::create_dir_all(root.join("thumbnails")).unwrap();
            for id in [FIRST, SECOND] {
                std::fs::write(root.join("videos").join(format!("{id}.mov")), b"movie").unwrap();
                std::fs::write(
                    root.join("thumbnails").join(format!("{id}.png")),
                    b"thumbnail",
                )
                .unwrap();
            }
        }
        (base, current, legacy)
    }
    #[test]
    fn exact_single_asset_and_legacy_cache_are_selected() {
        let (base, current, legacy) = fixture();
        for root in [&current, &legacy] {
            let movie = root.join("videos").join(format!("{FIRST}.mov"));
            let listing = format!(
                "p123\nf6\nn{}\nf7\nn{}\nn/dev/null\n",
                movie.display(),
                movie.display()
            );
            assert_eq!(
                select_thumbnail(&listing, &current, &legacy),
                Some(root.join("thumbnails").join(format!("{FIRST}.png")).canonicalize().unwrap())
            );
        }
        let next = current.join("videos").join(format!("{SECOND}.mov"));
        assert_eq!(select_thumbnail(&format!("n{}", next.display()), &current, &legacy),
            Some(current.join("thumbnails").join(format!("{SECOND}.png")).canonicalize().unwrap()));
        std::fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn ambiguity_missing_cache_and_outside_movies_do_not_guess() {
        let (base, current, legacy) = fixture();
        let movie = current.join("videos").join(format!("{FIRST}.mov"));
        let second = current.join("videos").join(format!("{SECOND}.mov"));
        assert_eq!(
            select_thumbnail(
                &format!("n{}\nn{}", movie.display(), second.display()),
                &current,
                &legacy
            ),
            None
        );
        assert_eq!(
            select_thumbnail(&format!("n/tmp/{FIRST}.mov"), &current, &legacy),
            None
        );
        std::fs::remove_file(current.join("thumbnails").join(format!("{FIRST}.png"))).unwrap();
        assert_eq!(
            select_thumbnail(&format!("n{}", movie.display()), &current, &legacy),
            None
        );
        assert_eq!(asset_id(Path::new("category.mov")), None);
        assert_eq!(asset_id(Path::new(&format!("{FIRST}.png"))), None);
        std::fs::remove_dir_all(base).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn escaping_symlink_is_rejected() {
        let (base, current, legacy) = fixture();
        let movie = current.join("videos").join(format!("{FIRST}.mov"));
        std::fs::remove_file(&movie).unwrap();
        let outside = base.join("outside.mov");
        std::fs::write(&outside, b"movie").unwrap();
        std::os::unix::fs::symlink(outside, &movie).unwrap();
        assert_eq!(
            select_thumbnail(&format!("n{}", movie.display()), &current, &legacy),
            None
        );
        std::fs::remove_dir_all(base).unwrap();
    }
}

/// Read only the current all-Spaces provider; a shuffle category is not an asset.
pub fn aerial_selected() -> bool {
    let Some(home) = std::env::var_os("HOME") else { return false };
    let path = PathBuf::from(home).join("Library/Application Support/com.apple.wallpaper/Store/Index.plist");
    let Ok(file) = std::fs::File::open(path) else { return false };
    let Ok(meta) = file.metadata() else { return false };
    if meta.len() > 2 * 1024 * 1024 { return false; }
    use std::io::Read as _;
    let mut bytes = Vec::new();
    if file.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes).is_err() || bytes.len() > 2 * 1024 * 1024 { return false; }
    let Ok(value) = plist::Value::from_reader(std::io::Cursor::new(bytes)) else { return false };
    selected_provider(&value)
}
fn selected_provider(value: &plist::Value) -> bool {
    let choice = (|| {
        value.as_dictionary()?.get("AllSpacesAndDisplays")?.as_dictionary()?
            .get("Linked")?.as_dictionary()?.get("Content")?.as_dictionary()?
            .get("Choices")?.as_array()
    })();
    choice.is_some_and(|choices| !choices.is_empty() && choices.iter().all(|choice| {
        choice.as_dictionary().and_then(|d| d.get("Provider")).and_then(plist::Value::as_string)
            == Some("com.apple.wallpaper.choice.aerials")
    }))
}
#[cfg(test)]
mod provider_tests {
    #[test]
    fn static_or_missing_provider_never_uses_retained_aerial_process() {
        for (provider, expected) in [("com.apple.wallpaper.choice.aerials", true), ("com.apple.wallpaper.choice.image", false)] {
            let xml = format!("<plist version=\"1.0\"><dict><key>AllSpacesAndDisplays</key><dict><key>Linked</key><dict><key>Content</key><dict><key>Choices</key><array><dict><key>Provider</key><string>{provider}</string></dict></array></dict></dict></dict></dict></plist>");
            let value=plist::Value::from_reader(std::io::Cursor::new(xml)).unwrap();
            assert_eq!(super::selected_provider(&value), expected);
        }
        assert!(!super::selected_provider(&plist::Value::Dictionary(Default::default())));
    }
}
