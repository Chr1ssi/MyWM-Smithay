//! Notification pictures: raw `image-data`, picture files and icons of the icon theme, decoded and scaled to fit
//! `IMAGE_SIZE` (straight-alpha RGBA).
//!
//! Icons are looked up as the icon theme specification says, in a simple form: the user's theme
//! (`gtk-icon-theme-name` of GTK's `settings.ini`, which Qt's qt6ct is usually set to as well), the themes it
//! inherits, `hicolor`, then the `pixmaps` directories. Of a theme's directories the scalable ones win, else the
//! size closest to the 64 pixels Quickshell asked for.
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use mywm_shell_protocol::{COVER_SIZE, IMAGE_SIZE, Image};

/// Pixels as a D-Bus `image-data` hint (`iiibiiay`) has them.
pub struct Raw<'a> {
    pub width: i32,
    pub height: i32,
    pub rowstride: i32,
    pub has_alpha: bool,
    pub bits_per_sample: i32,
    pub channels: i32,
    pub data: &'a [u8],
}

/// The picture of an `image-data` hint; `None` if its numbers do not fit its bytes.
pub fn from_raw(raw: &Raw) -> Option<Image> {
    let (width, height) = (u32::try_from(raw.width).ok()?, u32::try_from(raw.height).ok()?);
    let channels = usize::try_from(raw.channels).ok()?;
    let rowstride = usize::try_from(raw.rowstride).ok()?;
    if raw.bits_per_sample != 8 || width == 0 || height == 0 || !(3..=4).contains(&channels) || channels != 3 + usize::from(raw.has_alpha) {
        return None;
    }
    let row_bytes = width as usize * channels;
    if rowstride < row_bytes || raw.data.len() < rowstride * (height as usize - 1) + row_bytes {
        return None;
    }
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for row in raw.data.chunks(rowstride).take(height as usize) {
        for pixel in row[..row_bytes].chunks(channels) {
            rgba.extend_from_slice(&pixel[..3]);
            rgba.push(if channels == 4 { pixel[3] } else { 255 });
        }
    }
    image::RgbaImage::from_raw(width, height, rgba).map(|image| fit(image, IMAGE_SIZE))
}

/// The StatusNotifierItem pixmap (ARGB32 in network byte order) that best fits `size`, as Quickshell picks it: the
/// smallest at least that large, else the largest; scaled to fit `size`.
pub fn from_argb_pixmaps(pixmaps: &[(i32, i32, Vec<u8>)], size: u32) -> Option<Image> {
    let size = size as i32;
    let fits = |(w, h, _): &&(i32, i32, Vec<u8>)| *w >= size && *h >= size;
    let chosen = pixmaps
        .iter()
        .filter(fits)
        .min_by_key(|(w, h, _)| w * h)
        .or_else(|| pixmaps.iter().max_by_key(|(w, h, _)| w * h))?;
    let (width, height, data) = chosen;
    let (width, height) = (u32::try_from(*width).ok()?, u32::try_from(*height).ok()?);
    if width == 0 || height == 0 || data.len() < width as usize * height as usize * 4 {
        return None;
    }
    let rgba = data.chunks(4).take(width as usize * height as usize).flat_map(|p| [p[1], p[2], p[3], p[0]]).collect();
    image::RgbaImage::from_raw(width, height, rgba).map(|image| fit(image, size as u32))
}

/// The icon `name` (looked up also in `extra`, an item's own `IconThemePath`) or file at `location`, fitting `size`.
pub fn load_icon(location: &str, extra: &[PathBuf], size: u32) -> Option<Image> {
    if location.is_empty() {
        return None;
    }
    let path = match location.strip_prefix("file://") {
        Some(path) => PathBuf::from(unescape(path)),
        None if location.starts_with('/') => PathBuf::from(location),
        None => find_icon_in(location, extra)?,
    };
    load_file(&path, size)
}

/// A picture in a file format (`icon-data` of a menu entry: PNG), fitting `size`.
pub fn from_bytes(data: &[u8], size: u32) -> Option<Image> {
    let image = image::load_from_memory(data).ok()?.into_rgba8();
    Some(fit(image, size))
}

/// The picture at `location`: a path, a `file://` URL or the name of an icon.
pub fn load(location: &str) -> Option<Image> {
    if location.is_empty() {
        return None;
    }
    let path = match location.strip_prefix("file://") {
        Some(path) => PathBuf::from(unescape(path)),
        None if location.starts_with('/') => PathBuf::from(location),
        None => find_icon(location)?,
    };
    load_file(&path, IMAGE_SIZE)
}

/// `%20` and the like of a URL path.
fn unescape(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = bytes.get(index + 1..index + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[index], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                index += 3;
            }
            (byte, _) => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A player's cover at `url` (`file://` or a path), scaled down to just cover `COVER_SIZE`. Covers on the web
/// are not fetched.
pub fn load_cover(url: &str) -> Option<Image> {
    let path = match url.strip_prefix("file://") {
        Some(path) => PathBuf::from(unescape(path)),
        None if url.starts_with('/') => PathBuf::from(url),
        None => return None,
    };
    let image = image::ImageReader::open(&path).ok()?.with_guessed_format().ok()?.decode().ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    let scale = (f64::from(COVER_SIZE.0) / f64::from(width.max(1))).max(f64::from(COVER_SIZE.1) / f64::from(height.max(1)));
    let image = if scale < 1.0 {
        let size = |v: u32| ((f64::from(v) * scale).round() as u32).max(1);
        image::imageops::resize(&image, size(width), size(height), image::imageops::FilterType::CatmullRom)
    } else {
        image
    };
    Some(Image { width: image.width(), height: image.height(), rgba: image.into_raw() })
}

fn load_file(path: &Path, size: u32) -> Option<Image> {
    let svg = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz"));
    let image = if svg {
        render_svg(&std::fs::read(path).ok()?, size)?
    } else {
        image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?.into_rgba8()
    };
    Some(fit(image, size))
}

fn render_svg(data: &[u8], target: u32) -> Option<image::RgbaImage> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(data, &usvg::Options::default()).ok()?;
    let size = tree.size();
    let scale = target as f32 / size.width().max(size.height()).max(1.0);
    let (width, height) = ((size.width() * scale).round().max(1.0) as u32, (size.height() * scale).round().max(1.0) as u32);
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let rgba = pixmap.pixels().iter().flat_map(|p| {
        let c = p.demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    });
    image::RgbaImage::from_raw(width, height, rgba.collect())
}

/// `image` scaled down (never up) to fit `max` pixels each way, keeping its proportions.
fn fit(image: image::RgbaImage, max: u32) -> Image {
    let (width, height) = image.dimensions();
    let image = if width.max(height) > max {
        let scale = f64::from(max) / f64::from(width.max(height));
        let (w, h) = (((f64::from(width) * scale).round() as u32).max(1), ((f64::from(height) * scale).round() as u32).max(1));
        image::imageops::resize(&image, w, h, image::imageops::FilterType::CatmullRom)
    } else {
        image
    };
    Image { width: image.width(), height: image.height(), rgba: image.into_raw() }
}

/// `$XDG_DATA_HOME` and `$XDG_DATA_DIRS`, most important first.
fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
    let dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    data_home.into_iter().chain(dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from)).collect()
}

/// Where icon themes live: `~/.icons`, then `icons` in every data directory.
fn icon_bases() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".icons"));
    home.into_iter().chain(data_dirs().into_iter().map(|d| d.join("icons"))).collect()
}

/// The user's icon theme as GTK's settings name it.
fn user_theme() -> Option<String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    let text = std::fs::read_to_string(config.join("gtk-3.0/settings.ini")).ok()?;
    ini_value(&text, "Settings", "gtk-icon-theme-name")
}

/// `key` of `[section]` in an ini-style file.
fn ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut current = "";
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name;
        } else if current == section
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            return Some(v.trim().to_owned());
        }
    }
    None
}

/// A directory of a theme (relative to the theme's directory in every base) with what `index.theme` says of it.
struct ThemeDir {
    path: PathBuf,
    size: u32,
    scalable: bool,
}

/// The directories of `theme` and the themes it inherits, from the first `index.theme` of it in `bases`. Its
/// icons may lie in any base (on NixOS every package has its own `hicolor` directories, without an index).
fn theme_dirs(bases: &[PathBuf], theme: &str) -> Option<(Vec<ThemeDir>, Vec<String>)> {
    let index = bases.iter().find_map(|base| std::fs::read_to_string(base.join(theme).join("index.theme")).ok())?;
    let list = |key| ini_value(&index, "Icon Theme", key).unwrap_or_default();
    let names = format!("{},{}", list("Directories"), list("ScaledDirectories"));
    let dirs = names
        .split(',')
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(|name| {
            let size = ini_value(&index, name, "Size").and_then(|s| s.parse().ok()).unwrap_or(0);
            let scale: u32 = ini_value(&index, name, "Scale").and_then(|s| s.parse().ok()).unwrap_or(1);
            let scalable = ini_value(&index, name, "Type").is_some_and(|t| t == "Scalable");
            ThemeDir { path: PathBuf::from(name), size: size * scale.max(1), scalable }
        })
        .collect();
    let inherits = list("Inherits").split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned).collect();
    Some((dirs, inherits))
}

/// The file of the icon `name`.
pub fn find_icon(name: &str) -> Option<PathBuf> {
    find_icon_in(name, &[])
}

/// The file of the icon `name`, looked for first right in `extra` and in the themes there, then as `find_icon`.
pub fn find_icon_in(name: &str, extra: &[PathBuf]) -> Option<PathBuf> {
    if name.contains('/') {
        return None;
    }
    let direct = extra.iter().flat_map(|dir| ["svg", "png"].map(|e| dir.join(format!("{name}.{e}")))).find(|f| f.is_file());
    if direct.is_some() {
        return direct;
    }
    let bases: Vec<PathBuf> = extra.iter().cloned().chain(icon_bases()).collect();
    // The user's theme and the themes it inherits, breadth first; hicolor always last.
    let mut queue: Vec<String> = user_theme().filter(|t| t != "hicolor").into_iter().collect();
    let mut seen = HashSet::new();
    let mut index = 0;
    while index <= queue.len() {
        let theme = queue.get(index).cloned().unwrap_or_else(|| "hicolor".into());
        index += 1;
        if !seen.insert(theme.clone()) {
            continue;
        }
        let Some((dirs, inherits)) = theme_dirs(&bases, &theme) else { continue };
        queue.extend(inherits.into_iter().filter(|t| t != "hicolor"));
        let mut best: Option<(u32, PathBuf)> = None;
        for base in &bases {
            for dir in &dirs {
                for extension in ["svg", "png"] {
                    let file = base.join(&theme).join(&dir.path).join(format!("{name}.{extension}"));
                    if !file.is_file() {
                        continue;
                    }
                    // Scalable first, then the size nearest 64; larger ones count half as far off.
                    let distance = if dir.scalable || extension == "svg" { 0 } else if dir.size >= 64 { dir.size - 64 } else { 2 * (64 - dir.size) };
                    if best.as_ref().is_none_or(|(d, _)| distance < *d) {
                        best = Some((distance, file));
                    }
                }
            }
        }
        if let Some((_, file)) = best {
            return Some(file);
        }
    }
    data_dirs().into_iter().flat_map(|d| ["png", "svg", "xpm"].map(|e| d.join("pixmaps").join(format!("{name}.{e}")))).find(|f| f.is_file())
}

/// The icon a desktop entry (`desktop-entry` hint: `firefox` for `firefox.desktop`) names.
pub fn desktop_entry_icon(entry: &str) -> Option<String> {
    if entry.is_empty() || entry.contains('/') {
        return None;
    }
    let entry = entry.strip_suffix(".desktop").unwrap_or(entry);
    data_dirs().into_iter().find_map(|dir| {
        let text = std::fs::read_to_string(dir.join("applications").join(format!("{entry}.desktop"))).ok()?;
        ini_value(&text, "Desktop Entry", "Icon").filter(|icon| !icon.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_pixels_become_rgba() {
        // Two RGB pixels per row and a padded row stride.
        let data = [1, 2, 3, 4, 5, 6, 0, 0, 7, 8, 9, 10, 11, 12, 0, 0];
        let raw = Raw { width: 2, height: 2, rowstride: 8, has_alpha: false, bits_per_sample: 8, channels: 3, data: &data };
        let image = from_raw(&raw).expect("picture");
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(image.rgba, [1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255]);
        let short = Raw { height: 3, ..raw };
        assert!(from_raw(&short).is_none());
    }

    #[test]
    fn tray_pixmaps_are_argb_and_the_best_size_wins() {
        let pixmap = |size: i32, byte: u8| (size, size, [255, byte, 0, 0].repeat((size * size) as usize));
        let image = from_argb_pixmaps(&[pixmap(16, 1), pixmap(48, 2), pixmap(64, 3)], 36).expect("icon");
        // 48 is the smallest at least 36 large; scaled to 36, red from byte 1 of ARGB.
        assert_eq!((image.width, image.height), (36, 36));
        assert_eq!(&image.rgba[..4], &[2, 0, 0, 255]);
        let only_small = from_argb_pixmaps(&[pixmap(16, 9)], 36).expect("icon");
        assert_eq!((only_small.width, only_small.rgba[0]), (16, 9));
        assert!(from_argb_pixmaps(&[], 36).is_none());
    }

    #[test]
    fn large_pictures_shrink_to_fit() {
        let image = fit(image::RgbaImage::new(512, 256), IMAGE_SIZE);
        assert_eq!((image.width, image.height), (IMAGE_SIZE, IMAGE_SIZE / 2));
        let small = fit(image::RgbaImage::new(16, 8), IMAGE_SIZE);
        assert_eq!((small.width, small.height), (16, 8));
    }

    #[test]
    fn icons_come_from_the_theme_and_its_ancestors() {
        let root = std::env::temp_dir().join(format!("mywm-icons-{}", std::process::id()));
        let theme = root.join("share/icons/Mine");
        let hicolor = root.join("share/icons/hicolor");
        std::fs::create_dir_all(theme.join("scalable/apps")).unwrap();
        std::fs::create_dir_all(hicolor.join("48x48/apps")).unwrap();
        std::fs::create_dir_all(hicolor.join("128x128/apps")).unwrap();
        std::fs::write(theme.join("index.theme"), "[Icon Theme]\nInherits=hicolor\nDirectories=scalable/apps\n[scalable/apps]\nSize=48\nType=Scalable\n").unwrap();
        std::fs::write(
            hicolor.join("index.theme"),
            "[Icon Theme]\nDirectories=48x48/apps,128x128/apps\n[48x48/apps]\nSize=48\n[128x128/apps]\nSize=128\n",
        )
        .unwrap();
        std::fs::write(theme.join("scalable/apps/own.svg"), "").unwrap();
        std::fs::write(hicolor.join("48x48/apps/app.png"), "").unwrap();
        std::fs::write(hicolor.join("128x128/apps/app.png"), "").unwrap();
        std::fs::create_dir_all(root.join("config/gtk-3.0")).unwrap();
        std::fs::write(root.join("config/gtk-3.0/settings.ini"), "[Settings]\ngtk-icon-theme-name=Mine\n").unwrap();
        // SAFETY: this test is the only one reading these variables.
        unsafe {
            std::env::set_var("XDG_DATA_HOME", root.join("share"));
            std::env::set_var("XDG_DATA_DIRS", root.join("none"));
            std::env::set_var("XDG_CONFIG_HOME", root.join("config"));
            std::env::set_var("HOME", root.join("home"));
        }
        assert_eq!(find_icon("own"), Some(theme.join("scalable/apps/own.svg")));
        // 48 is 16 below 64 (counted double), 128 is 64 above.
        assert_eq!(find_icon("app"), Some(hicolor.join("48x48/apps/app.png")));
        assert_eq!(find_icon("missing"), None);
        assert_eq!(unescape("/a%20b/c"), "/a b/c");
        std::fs::remove_dir_all(root).unwrap();
    }
}
