//! Site marks are presentation only: no browser session, credential store, or third-party service.
use gpui_kit::{assets::IconName, component::{ActiveTheme as _, Icon, Sizable}, prelude::*, *};
use std::{collections::VecDeque, io::Read, sync::{Arc, Mutex, OnceLock}};

const MAX_BYTES: u64 = 256 * 1024;
type Cache = VecDeque<(String, Option<Arc<Image>>)>;
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
// This mutex bounds network/decode work to one request at a time, away from the UI thread.
static FETCH: Mutex<()> = Mutex::new(());

fn favicon_url(value: &str) -> Option<String> {
    monitter_lib::experimental_desktop::validate_native_workspace_url(value).ok()?;
    let mut url = url::Url::parse(value).ok()?;
    url.set_path("/favicon.ico"); url.set_query(None); url.set_fragment(None);
    Some(url.into())
}
fn tint(bytes: &[u8], color: Rgba) -> Option<Arc<Image>> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(512); limits.max_image_height = Some(512);
    limits.max_alloc = Some(4 * 1024 * 1024); reader.limits(limits);
    let mut image = reader.decode().ok()?.thumbnail(32, 32).to_rgba8();
    colorize(&mut image, color);
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image).write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(Arc::new(Image::from_bytes(ImageFormat::Png, png.into_inner())))
}
fn colorize(image: &mut image::RgbaImage, color: Rgba) {
    for pixel in image.pixels_mut() {
        let luminance = (0.2126 * pixel[0] as f32 + 0.7152 * pixel[1] as f32 + 0.0722 * pixel[2] as f32) / 255.;
        // Colourise midtones while keeping highlights, shadows, and transparency recognisable.
        for (channel, project) in [color.r, color.g, color.b].into_iter().enumerate() {
            let value = if luminance < 0.5 { project * luminance * 2. }
                else { project + (1. - project) * (luminance - 0.5) * 2. };
            pixel[channel] = (value * 255.).round() as u8;
        }
    }
}
fn fetch(url: String, color: Rgba) -> Option<Arc<Image>> {
    let key = format!("{url}:{:.4}:{:.4}:{:.4}", color.r, color.g, color.b);
    let _guard = FETCH.lock().ok()?;
    let cache = CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
    if let Some((_, result)) = cache.lock().ok()?.iter().find(|(cached, _)| cached == &key) {
        return result.clone();
    }
    let result = (|| {
        // The supplied page can declare a relative icon. Root favicon remains the fallback.
        let mut root = favicon_url(&url)?;
        if let Some((bytes, final_page)) = download(&url) {
            root = favicon_url(final_page.as_str())?;
            if let Ok(html) = String::from_utf8(bytes) {
                for icon in declared_icons(&html, &final_page).into_iter().take(2) {
                    if let Some(image) = download(icon.as_str()).and_then(|(bytes, _)| tint(&bytes, color)) { return Some(image); }
                }
            }
        }
        download(&root).and_then(|(bytes, _)| tint(&bytes, color))
    })();
    if let Ok(mut cache) = cache.lock() {
        if cache.len() >= 48 { cache.pop_front(); }
        cache.push_back((key, result.clone()));
    }
    result
}
fn download(value: &str) -> Option<(Vec<u8>, url::Url)> {
    let mut url = url::Url::parse(value).ok()?;
    let started = std::time::Instant::now();
    for redirect in 0..=3 {
        // Each redirect is independently validated: no credentials, file URLs, or
        // cookie forwarding. The entire chain shares a five-second deadline.
        monitter_lib::experimental_desktop::validate_native_workspace_url(url.as_str()).ok()?;
        let remaining = 5. - started.elapsed().as_secs_f64();
        if remaining <= 0. { return None; }
        let timeout = format!("{remaining:.3}");
        let mut child = std::process::Command::new("/usr/bin/curl")
            .args(["-q", "--silent", "--include", "--proto", "=http,https", "--connect-timeout", "2",
                "--max-time", &timeout, "--max-filesize", "262144", "--", url.as_str()])
            .stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped()).spawn().ok()?;
        let mut bytes = Vec::new();
        let read = child.stdout.take()?.take(MAX_BYTES + 16385).read_to_end(&mut bytes);
        if read.is_err() || bytes.len() as u64 > MAX_BYTES + 16384 {
            let _ = child.kill(); let _ = child.wait(); return None;
        }
        let exit = child.wait();
        #[cfg(test)] eprintln!("favicon curl {url}: exit={exit:?}, bytes={}", bytes.len());
        if !exit.ok()?.success() { return None; }
        let mut body = bytes.as_slice();
        let (status, headers, content) = loop {
            let end = body.windows(4).position(|part| part == b"\r\n\r\n")?;
            if end > 16384 { return None; }
            let headers = std::str::from_utf8(&body[..end]).ok()?;
            let first = headers.lines().next()?;
            let status: u16 = first.split_ascii_whitespace().nth(1)?.parse().ok()?;
            let rest = &body[end + 4..];
            // HTTPS proxy CONNECT and informational responses precede the real headers.
            if status < 200 || first.to_ascii_lowercase().contains("connection established") { body = rest; continue; }
            break (status, headers, rest);
        };
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            if redirect == 3 { return None; }
            let location = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("location").then(|| value.trim())
            })?;
            url = url.join(location).ok()?;
        } else {
            return (status >= 200 && status < 300 && content.len() as u64 <= MAX_BYTES)
                .then(|| (content.to_vec(), url));
        }
    }
    None
}
// Bounded HTML metadata scanning: quoted/unquoted attributes, either order/case.
fn declared_icons(html: &str, page: &url::Url) -> Vec<url::Url> {
    let mut icons = Vec::new();
    let lower = html.to_ascii_lowercase();
    let mut offset = 0;
    while let Some(found) = lower[offset..].find("<link") {
        let start = offset + found + 5;
        if !lower.as_bytes().get(start).is_some_and(|b| b.is_ascii_whitespace()) { offset = start; continue; }
        let mut end = start;
        let mut quote = None;
        for (i, character) in html[start..].char_indices() {
            match (quote, character) {
                (Some(q), c) if c == q => quote = None,
                (None, '\'' | '"') => quote = Some(character),
                (None, '>') => { end = start + i; break; },
                _ => {},
            }
        }
        if end == start { break; }
        offset = end + 1;
        let attributes = attributes(&html[start..end]);
        let rel = attributes.iter().find(|(name, _)| name == "rel").map(|(_, value)| value);
        if !rel.is_some_and(|rel| rel.split_ascii_whitespace().any(|word| word.eq_ignore_ascii_case("icon"))) { continue; }
        let Some((_, href)) = attributes.iter().find(|(name, _)| name == "href") else { continue; };
        let href = href.replace("&amp;", "&");
        let Ok(url) = page.join(&href) else { continue; };
        // SVG-only marks use the selected icon fallback. Prefer supported raster declarations.
        let svg_type = attributes.iter().any(|(key, value)| key == "type" && value.eq_ignore_ascii_case("image/svg+xml"));
        if svg_type || url.path().to_ascii_lowercase().ends_with(".svg") { continue; }
        if monitter_lib::experimental_desktop::validate_native_workspace_url(url.as_str()).is_ok() && !icons.contains(&url) {
            icons.push(url);
            if icons.len() >= 8 { break; }
        }
    }
    icons
}
fn attributes(mut value: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    while !value.is_empty() {
        value = value.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '/');
        let end = value.find(|c: char| c.is_ascii_whitespace() || c == '=').unwrap_or(value.len());
        if end == 0 { break; }
        let name = value[..end].to_ascii_lowercase();
        value = value[end..].trim_start();
        if !value.starts_with('=') { continue; }
        value = value[1..].trim_start();
        let Some(first) = value.chars().next() else { break; };
        let field;
        if matches!(first, '\'' | '"') {
            value = &value[1..];
            let Some(end) = value.find(first) else { break; };
            field = value[..end].to_owned(); value = &value[end + 1..];
        } else {
            let end = value.find(char::is_whitespace).unwrap_or(value.len());
            field = value[..end].to_owned(); value = &value[end..];
        }
        result.push((name, field));
    }
    result
}
pub(super) struct WorkspaceFavicon { key: Option<(String, Rgba)>, image: Option<Arc<Image>>, fallback: IconName, color: Rgba }
impl Render for WorkspaceFavicon {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = crate::design::workspace_tab_metrics(cx.theme().font_size).icon_size;
        match &self.image {
            Some(image) => img(image.clone()).size(size).into_any_element(),
            None => Icon::new(self.fallback.clone()).with_size(size).text_color(self.color).into_any_element(),
        }
    }
}
pub(super) fn workspace_favicon(id: &str, url: Option<&str>, fallback: IconName, color: Rgba,
    window: &mut Window, cx: &mut App) -> Entity<WorkspaceFavicon> {
    let holder = window.use_keyed_state(SharedString::from(format!("workspace-favicon-{id}")), cx,
        |_, cx| cx.new(|_| WorkspaceFavicon { key: None, image: None, fallback: fallback.clone(), color }));
    let entity = holder.read(cx).clone();
    let key = url.filter(|url| monitter_lib::experimental_desktop::validate_native_workspace_url(url).is_ok())
        .map(|url| (url.to_owned(), color));
    entity.update(cx, |view, cx| {
        if view.fallback != fallback || view.color != color { cx.notify(); }
        view.fallback = fallback; view.color = color;
        if view.key == key { return; }
        view.key = key.clone(); view.image = None;
        if let Some((url, color)) = key {
            let pending = view.key.clone();
            let task = cx.background_executor().spawn(async move { fetch(url, color) });
            cx.spawn(async move |view, cx| {
                let image = task.await;
                let _ = view.update(cx, |view, cx| {
                    if view.key == pending { view.image = image; cx.notify(); }
                });
            }).detach();
        }
        cx.notify();
    });
    entity
}
#[cfg(test)]
mod tests {
    use ::core::prelude::v1::test;
    use super::*;
    #[test]
    fn targets_origin_without_private_page_data() {
        assert_eq!(favicon_url("https://example.com/private?q=secret#token").as_deref(), Some("https://example.com/favicon.ico"));
        for url in ["file:///tmp/icon.png", "https://user:password@example.com", "javascript:alert(1)"] { assert!(favicon_url(url).is_none()); }
    }
    #[test]
    fn discovers_declared_relative_icons_and_rejects_credentials() {
        let page = url::Url::parse("https://example.com/project/").unwrap();
        let html = "<LINK href='../mark.png?a=1&amp;b=2' type='image/png' REL='shortcut ICON'>";
        assert_eq!(declared_icons(html, &page)[0].as_str(), "https://example.com/mark.png?a=1&b=2");
        assert!(declared_icons("<link rel=icon href='https://user:pass@example.com/icon.png'>", &page).is_empty());
    }
    #[test]
    fn invalid_or_oversized_images_use_fallback() {
        assert!(tint(b"<html>not an icon</html>", rgb(0x38bfd2).into()).is_none());
        let image = image::DynamicImage::new_rgba8(513, 1);
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        assert!(tint(bytes.get_ref(), rgb(0x38bfd2).into()).is_none());
    }
    #[test]
    fn recolouring_preserves_alpha_and_luminance_detail() {
        let mut pixels = image::RgbaImage::new(4, 1);
        for (index, pixel) in [[0, 0, 0, 255], [64, 64, 64, 83], [128, 128, 128, 192], [255, 255, 255, 255]].into_iter().enumerate() {
            pixels.put_pixel(index as u32, 0, image::Rgba(pixel));
        }
        colorize(&mut pixels, rgb(0x38bfd2).into());
        assert_eq!(pixels.get_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(pixels.get_pixel(3, 0).0, [255, 255, 255, 255]);
        assert_eq!(pixels.get_pixel(1, 0)[3], 83);
        assert_eq!(pixels.get_pixel(2, 0)[3], 192);
        for channel in 0..3 { assert!(pixels.get_pixel(1, 0)[channel] < pixels.get_pixel(2, 0)[channel]); }
        assert!(pixels.get_pixel(2, 0)[0] < pixels.get_pixel(2, 0)[1]);
    }
    #[test]
    fn local_site_redirect_discovers_raster_icon_and_missing_mark_falls_back() {
        use std::{io::Write, net::TcpListener, sync::atomic::{AtomicBool, Ordering}};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stopping = Arc::new(AtomicBool::new(false));
        let requests_worker = requests.clone();
        let stopping_worker = stopping.clone();
        let mut icon = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(2, 2, image::Rgba([128, 128, 128, 160])))
            .write_to(&mut icon, image::ImageFormat::Png).unwrap();
        let worker = std::thread::spawn(move || {
            while !stopping_worker.load(Ordering::Relaxed) {
                let Ok((mut socket, _)) = listener.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(2)); continue;
                };
                socket.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
                let mut request_bytes = Vec::new();
                let mut chunk = [0u8; 1024];
                while request_bytes.len() < 16384 && !request_bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    match socket.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => request_bytes.extend_from_slice(&chunk[..count]),
                    }
                }
                let request = String::from_utf8_lossy(&request_bytes);
                let path = request.lines().next().and_then(|line| line.split_ascii_whitespace().nth(1)).unwrap_or("");
                assert!(!request.to_ascii_lowercase().contains("cookie:"));
                assert!(!request.to_ascii_lowercase().contains("authorization:"));
                requests_worker.lock().unwrap().push(path.to_owned());
                let (status, headers, body): (&str, &str, &[u8]) = match path {
                    "/start" => ("302 Found", "Location: /project/page\r\n", b""),
                    "/project/page" => ("200 OK", "Content-Type: text/html\r\n", b"<link rel=icon type='image/svg+xml' href='/mark.svg'><link rel=icon href='mark.png'>"),
                    "/project/mark.png" => ("200 OK", "Content-Type: image/png\r\n", icon.get_ref()),
                    "/missing" => ("200 OK", "Content-Type: text/html\r\n", b"<link rel=icon href='/absent.png'>"),
                    "/loop" => ("302 Found", "Location: /loop\r\n", b""),
                    "/unsafe" => ("302 Found", "Location: file:///tmp/private.png\r\n", b""),
                    _ => ("404 Not Found", "", b"missing"),
                };
                let response = format!("HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = socket.write_all(response.as_bytes()); let _ = socket.write_all(body);
            }
        });
        // Keep teardown reliable even if an assertion fails.
        struct ServerGuard(Arc<AtomicBool>, Option<std::thread::JoinHandle<()>>);
        impl Drop for ServerGuard { fn drop(&mut self) { self.0.store(true, Ordering::Relaxed); self.1.take().unwrap().join().unwrap(); } }
        let _server = ServerGuard(stopping, Some(worker));
        let base = format!("http://{address}");
        assert!(fetch(format!("{base}/start"), rgb(0x38bfd2).into()).is_some(), "requests: {:?}", requests.lock().unwrap());
        assert!(fetch(format!("{base}/missing"), rgb(0x38bfd2).into()).is_none());
        assert!(download(&format!("{base}/loop")).is_none());
        assert!(download(&format!("{base}/unsafe")).is_none());
        let requests = requests.lock().unwrap();
        assert!(requests.contains(&"/project/mark.png".to_owned()));
        assert!(!requests.contains(&"/mark.svg".to_owned()));
        assert!(requests.contains(&"/absent.png".to_owned()), "requests: {requests:?}");
        assert!(requests.contains(&"/favicon.ico".to_owned()));
        let redirects = requests.iter().filter(|path| *path == "/loop").count();
        assert!((1..=4).contains(&redirects), "redirect loop must stop within its request and time budgets");
    }

}
