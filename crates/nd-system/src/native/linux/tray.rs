//! StatusNotifierItem through `ksni`. Items are served whether or not a watcher runs yet
//! (`assume_sni_available`), so a tray created before the panel starts shows up once it does.

use std::collections::HashMap;
use std::sync::Mutex;

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};

use crate::backend::Emitter;
use crate::{Error, Result, SystemEvent, Tray, TrayIcon, TrayId, TrayMenuItem};

struct Item {
    tray: Tray,
    pixmap: Vec<ksni::Icon>,
    events: Emitter,
}

impl Item {
    fn new(tray: Tray, events: Emitter) -> Result<Self> {
        let pixmap = match &tray.icon {
            Some(TrayIcon::Png { bytes, .. }) => vec![argb_icon(bytes)?],
            _ => Vec::new(),
        };
        Ok(Self { tray, pixmap, events })
    }

    fn emit(&self, item: Option<String>) {
        self.events.emit(SystemEvent::Tray { tray: self.tray.id.clone(), item });
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
        match &self.tray.icon {
            Some(TrayIcon::Named(name)) => name.clone(),
            _ => String::new(),
        }
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
    pub fn set(&self, tray: Tray, events: Emitter) -> Result<()> {
        let key = tray.id.0.clone();
        let item = Item::new(tray, events)?;
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
