//! C ABI over brave/adblock-rust for the NativeDesktop hosts (Zig on Linux,
//! Swift on macOS). Every entry point catches panics: an unwind across the FFI
//! boundary is undefined behaviour, and a bad filter must not take the host down.

use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use adblock::Engine;
use adblock::lists::{FilterFormat, FilterSet, ParseOptions};
use adblock::request::Request;
use adblock::resources::Resource;

pub struct NdAbEngine {
    engine: Engine,
}

fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

unsafe fn str_arg<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

unsafe fn bytes_arg<'a>(p: *const u8, len: usize) -> &'a [u8] {
    if p.is_null() || len == 0 {
        return &[];
    }
    unsafe { std::slice::from_raw_parts(p, len) }
}

fn out_string(s: String) -> *mut c_char {
    CString::new(s).map(CString::into_raw).unwrap_or(ptr::null_mut())
}

pub const ND_AB_FORMAT_STANDARD: i32 = 0;
/// `/etc/hosts` lines (Peter Lowe's list as uBO fetches it).
pub const ND_AB_FORMAT_HOSTS: i32 = 1;

/// Compiles an engine from filter list text. `lists` holds `count` pointers to
/// NUL-terminated lists and `formats` one ND_AB_FORMAT_* per list; each is
/// parsed as its own list so `!#if` blocks and list metadata stay scoped to it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nd_ab_engine_from_lists(
    lists: *const *const c_char,
    formats: *const i32,
    count: usize,
) -> *mut NdAbEngine {
    guard(ptr::null_mut(), || {
        let mut set = FilterSet::new(false);
        for i in 0..count {
            let p = unsafe { *lists.add(i) };
            let format = if formats.is_null() { ND_AB_FORMAT_STANDARD } else { unsafe { *formats.add(i) } };
            let opts = ParseOptions {
                format: if format == ND_AB_FORMAT_HOSTS { FilterFormat::Hosts } else { FilterFormat::Standard },
                ..ParseOptions::default()
            };
            if let Some(text) = unsafe { str_arg(p) } {
                set.add_filter_list(text.to_owned(), opts);
            }
        }
        Box::into_raw(Box::new(NdAbEngine { engine: Engine::new_with_filter_set(set) }))
    })
}

/// Restores an engine from `nd_ab_engine_serialize` output. Returns NULL when
/// the bytes come from another adblock-rust version.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nd_ab_engine_deserialize(data: *const u8, len: usize) -> *mut NdAbEngine {
    guard(ptr::null_mut(), || {
        let mut engine = Engine::default();
        match engine.deserialize(unsafe { bytes_arg(data, len) }) {
            Ok(()) => Box::into_raw(Box::new(NdAbEngine { engine })),
            Err(_) => ptr::null_mut(),
        }
    })
}

/// Serializes the engine. Free `*out` with `nd_ab_bytes_free(*out, *out_len)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nd_ab_engine_serialize(e: *const NdAbEngine, out: *mut *mut u8, out_len: *mut usize) -> bool {
    guard(false, || {
        let Some(e) = (unsafe { e.as_ref() }) else { return false };
        let bytes = e.engine.serialize().into_boxed_slice();
        unsafe {
            *out_len = bytes.len();
            *out = Box::into_raw(bytes) as *mut u8;
        }
        true
    })
}

/// Loads scriptlet and redirect resources: a JSON array in adblock-rust's
/// `Resource` shape (brave/adblock-resources `dist/resources.json`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nd_ab_engine_use_resources(e: *mut NdAbEngine, json: *const u8, len: usize) -> bool {
    guard(false, || {
        let Some(e) = (unsafe { e.as_mut() }) else { return false };
        match serde_json::from_slice::<Vec<Resource>>(unsafe { bytes_arg(json, len) }) {
            Ok(resources) => {
                e.engine.use_resources(resources);
                true
            }
            Err(_) => false,
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nd_ab_engine_free(e: *mut NdAbEngine) {
    if !e.is_null() {
        let _ = guard((), || drop(unsafe { Box::from_raw(e) }));
    }
}

pub const ND_AB_ALLOW: i32 = 0;
pub const ND_AB_BLOCK: i32 = 1;
pub const ND_AB_REDIRECT: i32 = 2;
/// An exception rule matched: allow, and a later engine must not block either.
pub const ND_AB_EXCEPTED: i32 = 3;

/// Checks one network request. `request_type` uses adblock-rust's names
/// ("script", "image", "sub_frame", "xmlhttprequest", ...). On
/// ND_AB_REDIRECT, `*redirect` receives a `data:` URL to serve instead; free it
/// with `nd_ab_string_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nd_ab_check(
    e: *const NdAbEngine,
    url: *const c_char,
    source_url: *const c_char,
    request_type: *const c_char,
    redirect: *mut *mut c_char,
) -> i32 {
    guard(ND_AB_ALLOW, || {
        let Some(e) = (unsafe { e.as_ref() }) else { return ND_AB_ALLOW };
        let (Some(url), Some(source), Some(kind)) =
            (unsafe { str_arg(url) }, unsafe { str_arg(source_url) }, unsafe { str_arg(request_type) })
        else {
            return ND_AB_ALLOW;
        };
        let Ok(request) = Request::new(url, source, kind, "GET") else { return ND_AB_ALLOW };
        let result = e.engine.check_network_request(&request);
        let blocked = result.important || (result.filter.is_some() && result.exception.is_none());
        if !blocked {
            return if result.exception.is_some() { ND_AB_EXCEPTED } else { ND_AB_ALLOW };
        }
        match result.redirect {
            Some(data_url) if !redirect.is_null() => {
                unsafe { *redirect = out_string(data_url) };
                ND_AB_REDIRECT
            }
            _ => ND_AB_BLOCK,
        }
    })
}

/// Page-specific cosmetic resources for `url` as JSON:
/// `{"hide_selectors":[..],"procedural_actions":[..],"exceptions":[..],"injected_script":"..","generichide":bool}`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nd_ab_url_cosmetic(e: *const NdAbEngine, url: *const c_char) -> *mut c_char {
    guard(ptr::null_mut(), || {
        let (Some(e), Some(url)) = (unsafe { e.as_ref() }, unsafe { str_arg(url) }) else {
            return ptr::null_mut();
        };
        let r = e.engine.url_cosmetic_resources(url);
        let json = serde_json::json!({
            "hide_selectors": r.hide_selectors,
            "procedural_actions": r.procedural_actions,
            "exceptions": r.exceptions,
            "injected_script": r.injected_script,
            "generichide": r.generichide,
        });
        out_string(json.to_string())
    })
}

/// Generic hide selectors matching the classes and ids a page reported, as a
/// JSON array of selectors. All three arguments are JSON arrays of strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nd_ab_hidden_selectors(
    e: *const NdAbEngine,
    classes: *const c_char,
    ids: *const c_char,
    exceptions: *const c_char,
) -> *mut c_char {
    guard(ptr::null_mut(), || {
        let Some(e) = (unsafe { e.as_ref() }) else { return ptr::null_mut() };
        let parse = |p: *const c_char| -> Vec<String> {
            unsafe { str_arg(p) }.and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default()
        };
        let exceptions: std::collections::HashSet<String> = parse(exceptions).into_iter().collect();
        let selectors = e.engine.hidden_class_id_selectors(parse(classes), parse(ids), &exceptions);
        out_string(serde_json::to_string(&selectors).unwrap_or_else(|_| "[]".into()))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nd_ab_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nd_ab_bytes_free(p: *mut u8, len: usize) {
    if !p.is_null() {
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(p, len)) });
    }
}
