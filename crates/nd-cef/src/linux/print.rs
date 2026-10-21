//! Printing on Linux through the desktop portal (`org.freedesktop.portal.Print`): Chromium's
//! print handler shows the portal's dialog (`PreparePrint`), lays the page out on the paper the
//! user picked, renders it to a PDF and hands that to the portal (`Print`), which sends it to
//! the printer. The portal calls block, so each runs on a thread of its own and its answer comes
//! back to the main thread.

use std::cell::RefCell;
use std::collections::HashMap;
use std::os::raw::c_int;

use cef::*;
use zbus::zvariant::{OwnedValue, Value};

const DESKTOP: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const PRINT: &str = "org.freedesktop.portal.Print";
/// The resolution pages are laid out at, in device units per inch.
const DPI: i32 = 300;

/// Paper and orientation from the portal's page setup, in millimetres.
#[derive(Clone, Debug, PartialEq)]
struct Setup {
    width_mm: f64,
    height_mm: f64,
    margins_mm: [f64; 4],
    landscape: bool,
    copies: i32,
}

impl Default for Setup {
    /// US Letter until the user picks.
    fn default() -> Self {
        Self { width_mm: 215.9, height_mm: 279.4, margins_mm: [6.35; 4], landscape: false, copies: 1 }
    }
}

thread_local! {
    /// What the last dialog chose, with the token `Print` must carry.
    static CHOSEN: RefCell<HashMap<i32, (Setup, u32)>> = RefCell::default();
}

fn device(mm: f64) -> i32 {
    (mm / 25.4 * f64::from(DPI)).round() as i32
}

fn apply(setup: &Setup, settings: &mut PrintSettings) {
    let (w, h) = (device(setup.width_mm), device(setup.height_mm));
    let [top, right, bottom, left] = setup.margins_mm.map(device);
    let paper = Size { width: w, height: h };
    let printable = Rect { x: left, y: top, width: (w - left - right).max(1), height: (h - top - bottom).max(1) };
    settings.set_device_name(Some(&CefString::from("portal")));
    settings.set_dpi(DPI);
    settings.set_orientation(c_int::from(setup.landscape));
    settings.set_copies(setup.copies.max(1));
    settings.set_printer_printable_area(Some(&paper), Some(&printable), 0);
}

fn number(map: &HashMap<String, OwnedValue>, key: &str) -> Option<f64> {
    let v = map.get(key)?;
    f64::try_from(v)
        .ok()
        .or_else(|| String::try_from(v.try_clone().ok()?).ok().and_then(|s| s.parse().ok()))
}

fn text(map: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    String::try_from(map.get(key)?.try_clone().ok()?).ok()
}

/// The portal's `page-setup` and `settings` results as the paper Chromium lays out on.
fn setup_from(page: &HashMap<String, OwnedValue>, settings: &HashMap<String, OwnedValue>) -> Setup {
    let base = Setup::default();
    let landscape = text(page, "Orientation")
        .or_else(|| text(settings, "orientation"))
        .is_some_and(|o| o.contains("landscape"));
    Setup {
        width_mm: number(page, "Width").unwrap_or(base.width_mm),
        height_mm: number(page, "Height").unwrap_or(base.height_mm),
        margins_mm: [
            number(page, "MarginTop").unwrap_or(base.margins_mm[0]),
            number(page, "MarginRight").unwrap_or(base.margins_mm[1]),
            number(page, "MarginBottom").unwrap_or(base.margins_mm[2]),
            number(page, "MarginLeft").unwrap_or(base.margins_mm[3]),
        ],
        landscape,
        copies: text(settings, "n-copies").and_then(|c| c.parse().ok()).unwrap_or(1),
    }
}

/// A request's `handle_token`, unique in this process.
fn new_token() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(1);
    format!("nd_print_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
}

/// Calls `method` on the portal, whose options carry `token` as their `handle_token`, and waits
/// for its request's `Response`: `Ok(None)` when the user cancelled.
fn request<B>(method: &str, token: &str, body: &B) -> Result<Option<HashMap<String, OwnedValue>>, String>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let conn = zbus::blocking::Connection::session().map_err(|e| format!("session bus: {e}"))?;
    let sender = conn.unique_name().map(|n| n.trim_start_matches(':').replace('.', "_")).unwrap_or_default();
    let path = format!("{PATH}/request/{sender}/{token}");
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.portal.Request")
        .and_then(|b| b.member("Response"))
        .and_then(|b| b.path(path.as_str()))
        .map_err(|e| e.to_string())?
        .build();
    let mut responses =
        zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(4)).map_err(|e| e.to_string())?;
    conn.call_method(Some(DESKTOP), PATH, Some(PRINT), method, body)
        .map_err(|e| format!("{PRINT}.{method}: {e}"))?;
    let message = responses.next().ok_or("the portal closed the request")?.map_err(|e| e.to_string())?;
    let (code, results): (u32, HashMap<String, OwnedValue>) =
        message.body().deserialize().map_err(|e| format!("{method} response: {e}"))?;
    Ok((code == 0).then_some(results))
}

fn options(token: &str) -> HashMap<&'static str, Value<'_>> {
    HashMap::from([("handle_token", Value::from(token)), ("modal", Value::from(true))])
}

/// The print dialog: `done` gets the chosen setup and the token for `Print`, or `None` when
/// cancelled or no portal answered.
fn prepare(title: String, done: impl FnOnce(Option<(Setup, u32)>) + Send + 'static) {
    std::thread::spawn(move || {
        let empty: HashMap<&str, Value<'_>> = HashMap::new();
        let token = new_token();
        let answer = request("PreparePrint", &token, &("", title.as_str(), &empty, &empty, options(&token)));
        let chosen = match answer {
            Ok(Some(results)) => {
                let dict = |key: &str| -> HashMap<String, OwnedValue> {
                    results
                        .get(key)
                        .and_then(|v| v.try_clone().ok())
                        .and_then(|v| HashMap::<String, OwnedValue>::try_from(v).ok())
                        .unwrap_or_default()
                };
                let token = results.get("token").and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
                Some((setup_from(&dict("page-setup"), &dict("settings")), token))
            }
            Ok(None) => None,
            Err(e) => {
                tracing::warn!("nd-cef: print dialog: {e}");
                None
            }
        };
        done(chosen);
    });
}

/// Sends the rendered PDF to the portal, which prints it with the dialog's choices.
fn print(title: String, pdf: String, token: u32, done: impl FnOnce(Result<(), String>) + Send + 'static) {
    std::thread::spawn(move || {
        let result = std::fs::File::open(&pdf).map_err(|e| format!("{pdf}: {e}")).and_then(|file| {
            let fd = zbus::zvariant::Fd::from(&file);
            let handle = new_token();
            let mut options = options(&handle);
            options.insert("token", Value::from(token));
            request("Print", &handle, &("", title.as_str(), fd, options)).map(|_| ())
        });
        done(result);
    });
}

wrap_print_handler! {
    pub(crate) struct NdPrint;

    impl PrintHandler {
        fn on_print_start(&self, _browser: Option<&mut Browser>) {}

        fn on_print_settings(&self, browser: Option<&mut Browser>, settings: Option<&mut PrintSettings>, _get_defaults: c_int) {
            let Some(settings) = settings else { return };
            let id = browser.map(|b| b.identifier()).unwrap_or_default();
            let setup = CHOSEN.with(|c| c.borrow().get(&id).map(|(s, _)| s.clone())).unwrap_or_default();
            apply(&setup, settings);
        }

        fn on_print_dialog(&self, browser: Option<&mut Browser>, _has_selection: c_int, callback: Option<&mut PrintDialogCallback>) -> c_int {
            let (Some(browser), Some(callback)) = (browser, callback) else { return 0 };
            let id = browser.identifier();
            let title = crate::page::by_browser(id).map(|p| p.page.state().title).unwrap_or_default();
            let callback = Sendable(callback.clone());
            prepare(title, move |chosen| {
                crate::platform::on_main(move || {
                    let callback = callback.0;
                    match chosen {
                        Some((setup, token)) => {
                            CHOSEN.with(|c| c.borrow_mut().insert(id, (setup.clone(), token)));
                            match print_settings_create() {
                                Some(mut settings) => {
                                    apply(&setup, &mut settings);
                                    callback.cont(Some(&mut settings));
                                }
                                None => callback.cancel(),
                            }
                        }
                        None => callback.cancel(),
                    }
                });
            });
            1
        }

        fn on_print_job(&self, browser: Option<&mut Browser>, document_name: Option<&CefString>, pdf_file_path: Option<&CefString>, callback: Option<&mut PrintJobCallback>) -> c_int {
            let (Some(browser), Some(pdf), Some(callback)) = (browser, pdf_file_path, callback) else { return 0 };
            let id = browser.identifier();
            let Some((_, token)) = CHOSEN.with(|c| c.borrow().get(&id).cloned()) else { return 0 };
            let title = document_name.map(|d| d.to_string()).unwrap_or_default();
            let callback = Sendable(callback.clone());
            print(title, pdf.to_string(), token, move |result| {
                if let Err(e) = &result {
                    tracing::warn!("nd-cef: print job: {e}");
                }
                crate::platform::on_main(move || callback.0.cont());
            });
            1
        }

        fn on_print_reset(&self, browser: Option<&mut Browser>) {
            if let Some(browser) = browser {
                let id = browser.identifier();
                CHOSEN.with(|c| c.borrow_mut().remove(&id));
            }
        }

        fn pdf_paper_size(&self, _browser: Option<&mut Browser>, device_units_per_inch: c_int) -> Size {
            let setup = Setup::default();
            let scale = |mm: f64| (mm / 25.4 * f64::from(device_units_per_inch)).round() as c_int;
            Size { width: scale(setup.width_mm), height: scale(setup.height_mm) }
        }
    }
}

/// A CEF callback carried to another thread and back; CEF's callbacks are thread-safe
/// reference-counted objects.
struct Sendable<T>(T);

// SAFETY: see the type's docs; the value is only used again on the main thread.
unsafe impl<T> Send for Sendable<T> {}

pub(crate) fn handler() -> PrintHandler {
    NdPrint::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_setup_from_the_portal() {
        let owned = |v: Value<'_>| OwnedValue::try_from(v).unwrap();
        let page = HashMap::from([
            ("Width".to_owned(), owned(Value::from(210.0))),
            ("Height".to_owned(), owned(Value::from(297.0))),
            ("Orientation".to_owned(), owned(Value::from("landscape"))),
        ]);
        let settings = HashMap::from([("n-copies".to_owned(), owned(Value::from("2")))]);
        let setup = setup_from(&page, &settings);
        assert_eq!((setup.width_mm, setup.height_mm, setup.landscape, setup.copies), (210.0, 297.0, true, 2));
        assert_eq!(device(25.4), DPI);
    }
}
