//! Another process's on-screen windows as the window server lists them, and composited captures
//! of one: what `nd docs capture` uses on a launched example, so glass, the toolbar and the
//! traffic lights are in the picture. Reading window titles and capturing need Screen Recording
//! for the process that runs this (`ndshell grant`).

use std::ffi::c_void;
use std::path::Path;
use std::process::Command;

use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFString, CGRect};
use objc2_core_graphics::{
    CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo, CGWindowListOption, kCGNullWindowID,
    kCGWindowBounds, kCGWindowLayer, kCGWindowNumber, kCGWindowOwnerPID,
};

#[derive(Clone, Debug, PartialEq)]
pub struct WindowInfo {
    pub number: u32,
    /// Global screen points, origin top left.
    pub bounds: (f64, f64, f64, f64),
}

unsafe fn number(dict: &CFDictionary, key: &CFString) -> Option<i64> {
    let value = unsafe { dict.value((key as *const CFString).cast::<c_void>()) };
    let value = unsafe { value.cast::<CFNumber>().as_ref() }?;
    value.as_i64()
}

/// The normal-layer, on-screen windows of `pid`, front to back.
pub fn of_pid(pid: u32) -> Vec<WindowInfo> {
    let options = CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    let Some(list) = CGWindowListCopyWindowInfo(options, kCGNullWindowID) else { return Vec::new() };
    let list: &CFArray = &list;
    let mut out = Vec::new();
    for index in 0..list.count() {
        let dict = unsafe { list.value_at_index(index).cast::<CFDictionary>().as_ref() };
        let Some(dict) = dict else { continue };
        unsafe {
            if number(dict, kCGWindowOwnerPID) != Some(i64::from(pid)) || number(dict, kCGWindowLayer) != Some(0) {
                continue;
            }
            let Some(id) = number(dict, kCGWindowNumber) else { continue };
            let bounds = dict.value((kCGWindowBounds as *const CFString).cast::<c_void>()).cast::<CFDictionary>();
            let mut rect = CGRect::default();
            if !CGRectMakeWithDictionaryRepresentation(bounds.as_ref(), &mut rect) {
                continue;
            }
            out.push(WindowInfo {
                number: id as u32,
                bounds: (rect.origin.x, rect.origin.y, rect.size.width, rect.size.height),
            });
        }
    }
    out
}

/// A PNG of window `number` as the window server composites it, without its shadow.
pub fn capture(number: u32, png: &Path) -> Result<(), String> {
    let status = Command::new("/usr/sbin/screencapture")
        .args(["-x", "-o", &format!("-l{number}")])
        .arg(png)
        .status()
        .map_err(|e| format!("screencapture: {e}"))?;
    if !status.success() || !png.is_file() {
        return Err(format!(
            "screencapture -l{number} wrote nothing ({status}); does this process have Screen Recording?"
        ));
    }
    Ok(())
}
