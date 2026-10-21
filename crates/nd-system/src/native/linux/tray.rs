//! StatusNotifierItem through `ksni`. Items are served whether or not a watcher runs yet
//! (`assume_sni_available`), so a tray created before the panel starts shows up once it does.
//!
//! A named icon goes out only when an icon theme on this machine has it; otherwise the item
//! shows the app's own icon. Either way the file found also goes out as the pixmap with its
//! theme directory, because the panel looks names up in its own environment, which for a nix
//! install often lacks the app's `share/icons`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};

use crate::backend::Emitter;
use crate::{Error, Result, SystemEvent, Tray, TrayIcon, TrayId, TrayMenuItem};

struct Item {
    tray: Tray,
    icon_name: String,
    theme_path: String,
    pixmap: Vec<ksni::Icon>,
    events: Emitter,
}

impl Item {
    fn new(tray: Tray, app_id: &str, events: Emitter) -> Result<Self> {
        let (mut icon_name, mut theme_path, mut pixmap) = (String::new(), String::new(), Vec::new());
        match &tray.icon {
            Some(TrayIcon::Png { bytes, .. }) => pixmap = vec![argb_icon(bytes)?],
            Some(TrayIcon::Named(name)) => {
                let found = themed(name).map(|f| (name.as_str(), f)).or_else(|| themed(app_id).map(|f| (app_id, f)));
                match found {
                    Some((name, found)) => {
                        icon_name = name.to_owned();
                        theme_path = found.theme.to_string_lossy().into_owned();
                        pixmap = found.pixmap;
                    }
                    None => icon_name = name.clone(),
                }
            }
            None => {}
        }
        Ok(Self { tray, icon_name, theme_path, pixmap, events })
    }

    fn emit(&self, item: Option<String>) {
        self.events.emit(SystemEvent::Tray { tray: self.tray.id.clone(), item });
    }
}

/// An icon found in an icon theme: the `icons` directory holding that theme, and the file as a
/// pixmap when it is a PNG.
#[derive(Clone)]
struct Found {
    theme: PathBuf,
    pixmap: Vec<ksni::Icon>,
}

/// The data directories icons are looked up in: the app's own `share` beside its `bin` (a nix
/// store path or an install prefix), then the XDG ones.
fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(prefix) = std::env::current_exe().ok().and_then(|e| e.parent()?.parent().map(Path::to_path_buf)) {
        dirs.push(prefix.join("share"));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(d) => dirs.push(PathBuf::from(d)),
        None => dirs.extend(home.map(|h| h.join(".local/share"))),
    }
    let shared = std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.is_empty());
    dirs.extend(shared.as_deref().unwrap_or("/usr/local/share:/usr/share").split(':').map(PathBuf::from));
    dirs
}

/// The square size a theme directory is for (`48x48`, `scalable` as 0).
fn dir_size(path: &Path) -> Option<u32> {
    path.components().rev().find_map(|c| {
        let c = c.as_os_str().to_str()?;
        if c == "scalable" {
            return Some(0);
        }
        let (w, h) = c.split_once('x')?;
        (w == h).then(|| w.parse().ok()).flatten()
    })
}

/// `name` in an icon theme or `pixmaps`: a PNG nearest 64 px first, else an SVG. Cached, since
/// every unread count rebuilds the item.
fn themed(name: &str) -> Option<Found> {
    static CACHE: LazyLock<Mutex<HashMap<String, Option<Found>>>> = LazyLock::new(Mutex::default);
    if name.is_empty() || name.contains('/') {
        return None;
    }
    if let Some(found) = CACHE.lock().expect("icon cache").get(name) {
        return found.clone();
    }
    let found = data_dirs().into_iter().find_map(|dir| {
        let icons = dir.join("icons");
        let mut files = Vec::new();
        collect(&icons, name, 3, &mut files);
        let pixmap = dir.join("pixmaps").join(format!("{name}.png"));
        if pixmap.is_file() {
            files.push(pixmap);
        }
        let png = files
            .iter()
            .filter(|f| f.extension().is_some_and(|e| e == "png"))
            .min_by_key(|f| dir_size(f).map_or(u32::MAX, |s| s.abs_diff(64)));
        let file = png.or_else(|| files.first())?;
        let pixmap = Some(file)
            .filter(|f| f.extension().is_some_and(|e| e == "png"))
            .and_then(|f| argb_icon(&std::fs::read(f).ok()?).ok());
        Some(Found { theme: icons, pixmap: pixmap.into_iter().collect() })
    });
    CACHE.lock().expect("icon cache").insert(name.to_owned(), found.clone());
    found
}

/// Files named `name.png` or `name.svg` under `dir`, `depth` levels down (theme, size, context).
fn collect(dir: &Path, name: &str, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                collect(&path, name, depth - 1, out);
            }
        } else if path.file_stem().is_some_and(|s| s == name)
            && path.extension().is_some_and(|e| e == "png" || e == "svg")
        {
            out.push(path);
        }
    }
}

/// A PNG as the ARGB32 (network byte order) pixmap the spec carries.
fn argb_icon(png_bytes: &[u8]) -> Result<ksni::Icon> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut decoder = decoder;
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| Error::Invalid(format!("tray icon: {e}")))?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| Error::Invalid(format!("tray icon: {e}")))?;
    let pixels = &buf[..info.buffer_size()];
    let data = match info.color_type {
        png::ColorType::Rgba => pixels.chunks_exact(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect(),
        png::ColorType::Rgb => pixels.chunks_exact(3).flat_map(|p| [255, p[0], p[1], p[2]]).collect(),
        png::ColorType::GrayscaleAlpha => pixels.chunks_exact(2).flat_map(|p| [p[1], p[0], p[0], p[0]]).collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|&g| [255, g, g, g]).collect(),
        png::ColorType::Indexed => return Err(Error::Invalid("tray icon: indexed PNG after expansion".into())),
    };
    Ok(ksni::Icon { width: info.width as i32, height: info.height as i32, data })
}

fn menu(items: &[TrayMenuItem]) -> Vec<ksni::MenuItem<Item>> {
    items
        .iter()
        .map(|item| match item {
            TrayMenuItem::Separator => ksni::MenuItem::Separator,
            TrayMenuItem::Submenu { label, items } => {
                SubMenu { label: label.clone(), submenu: menu(items), ..Default::default() }.into()
            }
            TrayMenuItem::Item { id, label, enabled, checked: None } => {
                let id = id.clone();
                StandardItem {
                    label: label.clone(),
                    enabled: *enabled,
                    activate: Box::new(move |t: &mut Item| t.emit(Some(id.clone()))),
                    ..Default::default()
                }
                .into()
            }
            TrayMenuItem::Item { id, label, enabled, checked: Some(checked) } => {
                let id = id.clone();
                CheckmarkItem {
                    label: label.clone(),
                    enabled: *enabled,
                    checked: *checked,
                    activate: Box::new(move |t: &mut Item| t.emit(Some(id.clone()))),
                    ..Default::default()
                }
                .into()
            }
        })
        .collect()
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        self.tray.id.0.clone()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.emit(None);
    }

    fn title(&self) -> String {
        self.tray.title.clone().unwrap_or_default()
    }

    fn icon_name(&self) -> String {
        self.icon_name.clone()
    }

    fn icon_theme_path(&self) -> String {
        self.theme_path.clone()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.pixmap.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: self.tray.tooltip.clone().unwrap_or_default(), ..Default::default() }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        menu(&self.tray.menu)
    }
}

#[derive(Default)]
pub struct Trays(Mutex<HashMap<String, Handle<Item>>>);

impl Trays {
    pub fn set(&self, tray: Tray, app_id: &str, events: Emitter) -> Result<()> {
        let key = tray.id.0.clone();
        let item = Item::new(tray, app_id, events)?;
        let mut trays = self.0.lock().expect("trays");
        if let Some(handle) = trays.get(&key).filter(|h| !h.is_closed()) {
            handle.update(move |t| *t = item);
            return Ok(());
        }
        let handle = item.assume_sni_available(true).spawn().map_err(Error::platform)?;
        trays.insert(key, handle);
        Ok(())
    }

    pub fn remove(&self, id: &TrayId) -> Result<()> {
        let handle = self.0.lock().expect("trays").remove(&id.0).ok_or(Error::NotFound)?;
        handle.shutdown().wait();
        Ok(())
    }
}
