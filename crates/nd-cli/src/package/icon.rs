//! App icons from one square source PNG: `.icns` (macOS), sized PNGs (Linux), `.ico` (Windows).

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::imageops::FilterType;
use image::{ExtendedColorType, RgbaImage};

/// Loads `app.icon`. Icon Composer `.icon` bundles need Xcode's `actool` and are not read yet.
pub fn load(path: &Path) -> Result<RgbaImage> {
    if path.extension().is_some_and(|e| e == "icon") {
        bail!(
            "{}: Icon Composer bundles are not supported by nd package yet; point app.icon at a 1024 px PNG",
            path.display()
        );
    }
    let image = image::open(path).with_context(|| format!("reading icon {}", path.display()))?.into_rgba8();
    if image.width() != image.height() {
        bail!("{}: icon must be square, is {}x{}", path.display(), image.width(), image.height());
    }
    if image.width() < 512 {
        bail!("{}: icon must be at least 512 px (1024 recommended), is {}", path.display(), image.width());
    }
    Ok(image)
}

pub fn resized(image: &RgbaImage, size: u32) -> RgbaImage {
    if image.width() == size { image.clone() } else { image::imageops::resize(image, size, size, FilterType::Lanczos3) }
}

pub fn write_png(image: &RgbaImage, size: u32, out: &Path) -> Result<()> {
    resized(image, size).save(out).with_context(|| format!("writing {}", out.display()))
}

/// `iconutil` names of an `.iconset` and their pixel sizes.
pub const ICONSET: &[(&str, u32)] = &[
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
];

/// Writes `out` (`.icns`) through `iconutil`, macOS only.
pub fn write_icns(image: &RgbaImage, work: &Path, out: &Path) -> Result<()> {
    let set = work.join("AppIcon.iconset");
    std::fs::create_dir_all(&set)?;
    for (name, size) in ICONSET {
        write_png(image, *size, &set.join(name))?;
    }
    let status = Command::new("/usr/bin/iconutil")
        .args(["-c", "icns", "-o"])
        .arg(out)
        .arg(&set)
        .status()
        .context("running iconutil")?;
    if !status.success() {
        bail!("iconutil failed ({status})");
    }
    Ok(())
}

pub fn write_ico(image: &RgbaImage, out: &Path) -> Result<()> {
    let frames = [16u32, 24, 32, 48, 64, 128, 256]
        .iter()
        .map(|&size| IcoFrame::as_png(resized(image, size).as_raw(), size, size, ExtendedColorType::Rgba8))
        .collect::<Result<Vec<_>, _>>()?;
    let file = std::fs::File::create(out).with_context(|| format!("creating {}", out.display()))?;
    IcoEncoder::new(std::io::BufWriter::new(file)).encode_images(&frames)?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn test_png(path: &Path, size: u32) {
    let image = RgbaImage::from_fn(size, size, |x, y| image::Rgba([(x % 256) as u8, (y % 256) as u8, 200, 255]));
    image.save(path).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_non_square_and_bundle_icons_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let small = dir.path().join("small.png");
        test_png(&small, 64);
        assert!(load(&small).unwrap_err().to_string().contains("at least 512"));
        assert!(load(&dir.path().join("a.icon")).unwrap_err().to_string().contains("Icon Composer"));
    }

    #[test]
    fn ico_holds_every_size() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("icon.png");
        test_png(&src, 512);
        let out = dir.path().join("app.ico");
        write_ico(&load(&src).unwrap(), &out).unwrap();
        let bytes = std::fs::read(out).unwrap();
        assert_eq!(&bytes[..4], &[0, 0, 1, 0], "ICONDIR header");
        assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), 7);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn icns_is_built_by_iconutil() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("icon.png");
        test_png(&src, 1024);
        let out = dir.path().join("AppIcon.icns");
        write_icns(&load(&src).unwrap(), dir.path(), &out).unwrap();
        assert_eq!(&std::fs::read(out).unwrap()[..4], b"icns");
    }
}
