//! Self-contained browser documents for local raster images.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    fs::{self, OpenOptions},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};

const MAX_SOURCE: u64 = 20 * 1024 * 1024;
const MAX_PIXELS: u64 = 16 * 1024 * 1024;

pub(crate) fn prepare(path: &Path) -> Result<PathBuf, String> {
    let cache = monitter_lib::experimental_desktop::profile_dir()?.join("image-previews");
    prepare_in(path, &cache)
}

fn prepare_in(path: &Path, cache: &Path) -> Result<PathBuf, String> {
    let before = fs::metadata(path).map_err(|e| format!("Cannot inspect image: {e}"))?;
    if !before.is_file() || before.len() > MAX_SOURCE {
        return Err("Image must be a regular file no larger than 20 MiB.".into());
    }
    let file = fs::File::open(path).map_err(|e| format!("Cannot open image: {e}"))?;
    let metadata = file
        .metadata()
        .map_err(|e| format!("Cannot inspect image: {e}"))?;
    if !metadata.is_file() || metadata.len() > MAX_SOURCE {
        return Err("Image must be a regular file no larger than 20 MiB.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_SOURCE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read image: {e}"))?;
    if bytes.len() as u64 > MAX_SOURCE {
        return Err("Image exceeds the 20 MiB limit.".into());
    }
    let format = image::guess_format(&bytes).map_err(|e| format!("Unsupported image: {e}"))?;
    if !matches!(
        format,
        image::ImageFormat::Png
            | image::ImageFormat::Jpeg
            | image::ImageFormat::Gif
            | image::ImageFormat::WebP
            | image::ImageFormat::Bmp
            | image::ImageFormat::Tiff
            | image::ImageFormat::Ico
            | image::ImageFormat::Pnm
    ) {
        return Err("Unsupported image format.".into());
    }
    let (width, height) = image::ImageReader::with_format(Cursor::new(&bytes), format)
        .into_dimensions()
        .map_err(|e| format!("Cannot read image dimensions: {e}"))?;
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err(
            "Image dimensions exceed the safe preview limit (8192 px per side, 16 megapixels)."
                .into(),
        );
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|e| format!("Cannot decode image: {e}"))?;
    let mut png = Cursor::new(Vec::new());
    decoded
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| format!("Cannot prepare image: {e}"))?;
    let png = png.into_inner();
    let filename = path.file_name().unwrap_or_default().to_string_lossy();
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    digest.update(&(png.len() as u64).to_be_bytes());
    digest.update(&png);
    digest.update(filename.as_bytes());
    let hash = digest.finish();
    let name: String = hash.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    let title = escape_html(&filename);
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data:; style-src 'unsafe-inline'\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{title}</title><style>html,body{{margin:0;width:100%;height:100%;background:#f4f4f4}}@media (prefers-color-scheme:dark){{html,body{{background:#191919}}}}body{{display:flex;align-items:center;justify-content:center}}img{{display:block;max-width:100%;max-height:100%;object-fit:contain}}</style></head><body><img alt=\"{title}\" src=\"data:image/png;base64,{}\"></body></html>",
        STANDARD.encode(png)
    );
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(cache)
        .map_err(|e| format!("Cannot create image preview cache: {e}"))?;
    if !fs::symlink_metadata(cache)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_dir()
    {
        return Err("Image preview cache is not a directory.".into());
    }
    let destination = cache.join(format!("{name}.html"));
    // Publish a complete file atomically, without following an existing destination symlink.
    let temporary = cache.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = options
            .open(&temporary)
            .map_err(|e| format!("Cannot create image preview: {e}"))?;
        output
            .write_all(html.as_bytes())
            .map_err(|e| format!("Cannot write image preview: {e}"))?;
        drop(output);
        match fs::hard_link(&temporary, &destination) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(&destination).map_err(|e| e.to_string())?;
                if metadata.file_type().is_file() {
                    Ok(())
                } else {
                    Err("Image preview cache entry is not a regular file.".into())
                }
            }
            Err(e) => Err(format!("Cannot publish image preview: {e}")),
        }
    })();
    let _ = fs::remove_file(&temporary);
    result?;
    Ok(destination)
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp() -> PathBuf {
        let p = std::env::temp_dir().join(format!("monitter-image-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn png_and_jpeg_produce_safe_cached_html() {
        let dir = temp();
        for format in [image::ImageFormat::Png, image::ImageFormat::Jpeg] {
            let input = dir.join(format!("<evil\"&.{format:?}"));
            let mut data = Cursor::new(Vec::new());
            let mut image = image::RgbImage::new(2, 2);
            if format == image::ImageFormat::Jpeg {
                image.put_pixel(0, 0, image::Rgb([255, 0, 0]));
            }
            image::DynamicImage::ImageRgb8(image)
                .write_to(&mut data, format)
                .unwrap();
            fs::write(&input, data.into_inner()).unwrap();
            let result = prepare_in(&input, &dir.join("cache")).unwrap();
            let html = fs::read_to_string(&result).unwrap();
            assert!(html.contains("data:image/png;base64,iVBOR"));
            assert!(html.contains("&lt;evil&quot;&amp;"));
            assert!(html.contains("default-src 'none'"));
            assert_eq!(result, prepare_in(&input, &dir.join("cache")).unwrap());
        }
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn identical_images_keep_each_filename() {
        let dir = temp();
        let mut data = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut data, image::ImageFormat::Png)
            .unwrap();
        let first = dir.join("first.png");
        let second = dir.join("second.png");
        fs::write(&first, data.get_ref()).unwrap();
        fs::write(&second, data.get_ref()).unwrap();
        let cache = dir.join("cache");
        let a = prepare_in(&first, &cache).unwrap();
        let b = prepare_in(&second, &cache).unwrap();
        assert_ne!(a, b);
        assert!(
            fs::read_to_string(a)
                .unwrap()
                .contains("<title>first.png</title>")
        );
        assert!(
            fs::read_to_string(b)
                .unwrap()
                .contains("<title>second.png</title>")
        );
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_invalid_and_oversized_images() {
        let dir = temp();
        let input = dir.join("image");
        fs::write(&input, b"not an image").unwrap();
        assert!(prepare_in(&input, &dir.join("cache")).is_err());
        let file = fs::File::create(&input).unwrap();
        file.set_len(MAX_SOURCE + 1).unwrap();
        assert!(prepare_in(&input, &dir.join("cache")).is_err());
        let mut data = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(8193, 1)
            .write_to(&mut data, image::ImageFormat::Png)
            .unwrap();
        fs::write(&input, data.into_inner()).unwrap();
        assert!(prepare_in(&input, &dir.join("cache")).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
