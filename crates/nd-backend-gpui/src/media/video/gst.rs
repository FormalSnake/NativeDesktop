//! Linux: GStreamer's `playbin`, loaded with `dlopen` when a video first opens, so a system
//! without GStreamer still runs and the node reports "video unavailable". playbin demuxes,
//! decodes (gst-libav for H.264 and HEVC), plays the sound on the default sink and keeps the
//! clock. Its video sink is a bin that turns the picture upright, scales it to the box and ends
//! in an `appsink` this thread pulls BGRA frames from. No GLib main loop runs in an nd app, so
//! the thread pops the bus itself.

use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
use std::mem::offset_of;
use std::ptr;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, OnceLock};

use libloading::Library;

use super::{Command, Event, Frame, Shared, fit_size};

const STATE_NULL: c_int = 1;
const STATE_PAUSED: c_int = 3;
const STATE_PLAYING: c_int = 4;
const STATE_CHANGE_FAILURE: c_int = 0;
const FORMAT_TIME: c_int = 3;
const SEEK_FLUSH_ACCURATE: c_int = 1 | 1 << 1;
const MESSAGE_EOS: c_uint = 1;
const MESSAGE_ERROR: c_uint = 1 << 1;
const MESSAGE_ASYNC_DONE: c_uint = 1 << 21;
const MAP_READ: c_int = 1;
/// How long a pull waits for a frame before the thread looks at its commands and bus again.
const PULL_TIMEOUT_NS: u64 = 10_000_000;

const SINK: &CStr = c"videoconvert ! videoflip video-direction=auto ! videoscale ! \
    capsfilter name=fit caps=video/x-raw,format=BGRA,pixel-aspect-ratio=1/1 ! \
    appsink name=frames max-buffers=2 drop=true sync=true enable-last-sample=false";

/// The public head of `GstMiniObject`, frozen across GStreamer 1.x.
#[repr(C)]
struct GstMiniObject {
    type_: usize,
    refcount: c_int,
    lockstate: c_int,
    flags: c_uint,
    copy: *mut c_void,
    dispose: *mut c_void,
    free: *mut c_void,
    priv_uint: c_uint,
    priv_pointer: *mut c_void,
}

/// `GST_MESSAGE_TYPE` is a macro, so the type is read from the struct.
#[repr(C)]
struct GstMessage {
    mini_object: GstMiniObject,
    type_: c_uint,
}

/// `GST_BUFFER_PTS` is a macro too.
#[repr(C)]
struct GstBuffer {
    mini_object: GstMiniObject,
    pool: *mut c_void,
    pts: u64,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(offset_of!(GstMessage, type_) == 64 && offset_of!(GstBuffer, pts) == 72);

#[repr(C)]
struct GstMapInfo {
    memory: *mut c_void,
    flags: c_int,
    data: *const u8,
    size: usize,
    maxsize: usize,
    user_data: [*mut c_void; 4],
    reserved: [*mut c_void; 4],
}

#[repr(C)]
struct GError {
    domain: u32,
    code: c_int,
    message: *mut c_char,
}

type Obj = *mut c_void;

struct Gst {
    _libs: [Library; 4],
    element_factory_make: unsafe extern "C" fn(*const c_char, *const c_char) -> Obj,
    parse_bin_from_description: unsafe extern "C" fn(*const c_char, c_int, *mut *mut GError) -> Obj,
    bin_get_by_name: unsafe extern "C" fn(Obj, *const c_char) -> Obj,
    element_set_state: unsafe extern "C" fn(Obj, c_int) -> c_int,
    element_query_duration: unsafe extern "C" fn(Obj, c_int, *mut i64) -> c_int,
    element_seek_simple: unsafe extern "C" fn(Obj, c_int, c_int, i64) -> c_int,
    element_get_bus: unsafe extern "C" fn(Obj) -> Obj,
    bus_timed_pop_filtered: unsafe extern "C" fn(Obj, u64, c_uint) -> *mut GstMessage,
    message_parse_error: unsafe extern "C" fn(*mut GstMessage, *mut *mut GError, *mut *mut c_char),
    mini_object_unref: unsafe extern "C" fn(*mut c_void),
    object_ref_sink: unsafe extern "C" fn(Obj) -> Obj,
    object_unref: unsafe extern "C" fn(Obj),
    sample_get_buffer: unsafe extern "C" fn(Obj) -> *mut GstBuffer,
    sample_get_caps: unsafe extern "C" fn(Obj) -> Obj,
    caps_get_structure: unsafe extern "C" fn(Obj, c_uint) -> Obj,
    caps_from_string: unsafe extern "C" fn(*const c_char) -> Obj,
    structure_get_int: unsafe extern "C" fn(Obj, *const c_char, *mut c_int) -> c_int,
    buffer_map: unsafe extern "C" fn(*mut GstBuffer, *mut GstMapInfo, c_int) -> c_int,
    buffer_unmap: unsafe extern "C" fn(*mut GstBuffer, *mut GstMapInfo),
    filename_to_uri: unsafe extern "C" fn(*const c_char, *mut *mut GError) -> *mut c_char,
    uri_is_valid: unsafe extern "C" fn(*const c_char) -> c_int,
    app_sink_try_pull_sample: unsafe extern "C" fn(Obj, u64) -> Obj,
    app_sink_try_pull_preroll: unsafe extern "C" fn(Obj, u64) -> Obj,
    object_set: unsafe extern "C" fn(Obj, *const c_char, ...),
    error_free: unsafe extern "C" fn(*mut GError),
    free: unsafe extern "C" fn(*mut c_void),
}

fn gst() -> Result<&'static Gst, String> {
    static GST: OnceLock<Result<Gst, String>> = OnceLock::new();
    GST.get_or_init(Gst::load).as_ref().map_err(|e| format!("video unavailable: {e}"))
}

impl Gst {
    fn load() -> Result<Gst, String> {
        let open = |name: &str| {
            // SAFETY: these libraries' initializers have no preconditions.
            unsafe { Library::new(name) }.map_err(|e| format!("gstreamer not found ({e})"))
        };
        let gst = open("libgstreamer-1.0.so.0")?;
        let app = open("libgstapp-1.0.so.0")?;
        let gobject = open("libgobject-2.0.so.0")?;
        let glib = open("libglib-2.0.so.0")?;
        fn sym<T: Copy>(lib: &Library, name: &str) -> Result<T, String> {
            // SAFETY: each field's type is the C signature of the symbol it is loaded from.
            unsafe { lib.get::<T>(name.as_bytes()) }
                .map(|s| *s)
                .map_err(|e| format!("gstreamer is missing {name} ({e})"))
        }
        let init_check: unsafe extern "C" fn(*mut c_int, *mut c_void, *mut *mut GError) -> c_int =
            sym(&gst, "gst_init_check")?;
        let api = Gst {
            element_factory_make: sym(&gst, "gst_element_factory_make")?,
            parse_bin_from_description: sym(&gst, "gst_parse_bin_from_description")?,
            bin_get_by_name: sym(&gst, "gst_bin_get_by_name")?,
            element_set_state: sym(&gst, "gst_element_set_state")?,
            element_query_duration: sym(&gst, "gst_element_query_duration")?,
            element_seek_simple: sym(&gst, "gst_element_seek_simple")?,
            element_get_bus: sym(&gst, "gst_element_get_bus")?,
            bus_timed_pop_filtered: sym(&gst, "gst_bus_timed_pop_filtered")?,
            message_parse_error: sym(&gst, "gst_message_parse_error")?,
            mini_object_unref: sym(&gst, "gst_mini_object_unref")?,
            object_ref_sink: sym(&gst, "gst_object_ref_sink")?,
            object_unref: sym(&gst, "gst_object_unref")?,
            sample_get_buffer: sym(&gst, "gst_sample_get_buffer")?,
            sample_get_caps: sym(&gst, "gst_sample_get_caps")?,
            caps_get_structure: sym(&gst, "gst_caps_get_structure")?,
            caps_from_string: sym(&gst, "gst_caps_from_string")?,
            structure_get_int: sym(&gst, "gst_structure_get_int")?,
            buffer_map: sym(&gst, "gst_buffer_map")?,
            buffer_unmap: sym(&gst, "gst_buffer_unmap")?,
            filename_to_uri: sym(&gst, "gst_filename_to_uri")?,
            uri_is_valid: sym(&gst, "gst_uri_is_valid")?,
            app_sink_try_pull_sample: sym(&app, "gst_app_sink_try_pull_sample")?,
            app_sink_try_pull_preroll: sym(&app, "gst_app_sink_try_pull_preroll")?,
            object_set: sym(&gobject, "g_object_set")?,
            error_free: sym(&glib, "g_error_free")?,
            free: sym(&glib, "g_free")?,
            _libs: [gst, app, gobject, glib],
        };
        let mut error = ptr::null_mut();
        // SAFETY: GStreamer accepts null argc/argv.
        if unsafe { init_check(ptr::null_mut(), ptr::null_mut(), &mut error) } == 0 {
            return Err(format!("gstreamer failed to initialize: {}", api.take_error(error)));
        }
        Ok(api)
    }

    fn take_error(&self, error: *mut GError) -> String {
        if error.is_null() {
            return "unknown error".into();
        }
        // SAFETY: a GError from GStreamer, freed here once.
        unsafe {
            let message = (*error).message;
            let text = if message.is_null() {
                "unknown error".into()
            } else {
                CStr::from_ptr(message).to_string_lossy().into_owned()
            };
            (self.error_free)(error);
            text
        }
    }

    /// A URL as is, a path as a `file://` URI.
    fn uri(&self, source: &str) -> Result<CString, String> {
        let c = CString::new(source).map_err(|_| format!("not a path or URL: {source}"))?;
        // SAFETY: NUL-terminated strings; the returned URI is g_free'd after copying.
        unsafe {
            if (self.uri_is_valid)(c.as_ptr()) != 0 {
                return Ok(c);
            }
            let mut error = ptr::null_mut();
            let uri = (self.filename_to_uri)(c.as_ptr(), &mut error);
            if uri.is_null() {
                return Err(self.take_error(error));
            }
            let owned = CStr::from_ptr(uri).to_owned();
            (self.free)(uri.cast());
            Ok(owned)
        }
    }
}

pub(super) fn open(source: &str, shared: Arc<Shared>) -> Result<Sender<Command>, String> {
    let gst = gst()?;
    let uri = gst.uri(source)?;
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("nd-video".into())
        .spawn(move || {
            if let Err(error) = run(gst, &uri, &rx, &shared) {
                shared.event(Event::Failed(error));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(tx)
}

/// One playbin and the elements of its video sink this thread talks to.
struct Pipeline {
    gst: &'static Gst,
    playbin: Obj,
    fit_filter: Obj,
    frames: Obj,
    bus: Obj,
    playing: bool,
    looping: bool,
    native: Option<(u32, u32)>,
    fit: Option<(u32, u32, bool)>,
    /// The size the caps filter asks for now.
    asked: Option<(u32, u32)>,
    /// A preroll landed (opening, or a seek while paused) and its frame is not shown yet.
    prerolled: bool,
    /// At the end: the sink answers every pull at once until a seek.
    ended: bool,
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        // SAFETY: the pipeline owns one reference to each object.
        unsafe {
            (self.gst.element_set_state)(self.playbin, STATE_NULL);
            for obj in [self.fit_filter, self.frames, self.bus, self.playbin] {
                if !obj.is_null() {
                    (self.gst.object_unref)(obj);
                }
            }
        }
    }
}

fn run(gst: &'static Gst, uri: &CStr, commands: &Receiver<Command>, shared: &Shared) -> Result<(), String> {
    let mut p = Pipeline::new(gst, uri)?;
    loop {
        loop {
            match commands.try_recv() {
                Ok(command) => p.command(command),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
        p.bus(shared)?;
        p.pull(shared);
    }
}

impl Pipeline {
    fn new(gst: &'static Gst, uri: &CStr) -> Result<Pipeline, String> {
        // SAFETY: element construction on a loaded GStreamer; every object reference taken here
        // is released by Drop, including on the early returns.
        unsafe {
            let playbin = (gst.element_factory_make)(c"playbin".as_ptr(), ptr::null());
            if playbin.is_null() {
                return Err("gstreamer has no playbin (gst-plugins-base)".into());
            }
            let playbin = (gst.object_ref_sink)(playbin);
            let mut p = Pipeline {
                gst,
                playbin,
                fit_filter: ptr::null_mut(),
                frames: ptr::null_mut(),
                bus: (gst.element_get_bus)(playbin),
                playing: false,
                looping: false,
                native: None,
                fit: None,
                asked: None,
                prerolled: false,
                ended: false,
            };
            let mut error = ptr::null_mut();
            let sink = (gst.parse_bin_from_description)(SINK.as_ptr(), 1, &mut error);
            if sink.is_null() {
                return Err(format!("gstreamer video sink: {}", gst.take_error(error)));
            }
            p.fit_filter = (gst.bin_get_by_name)(sink, c"fit".as_ptr());
            p.frames = (gst.bin_get_by_name)(sink, c"frames".as_ptr());
            (gst.object_set)(playbin, c"video-sink".as_ptr(), sink, ptr::null::<c_char>());
            (gst.object_set)(playbin, c"uri".as_ptr(), uri.as_ptr(), ptr::null::<c_char>());
            if p.fit_filter.is_null() || p.frames.is_null() {
                return Err("gstreamer video sink is incomplete".into());
            }
            if (gst.element_set_state)(playbin, STATE_PAUSED) == STATE_CHANGE_FAILURE {
                let mut p = p;
                return Err(p.bus_error().unwrap_or_else(|| "the video could not be opened".into()));
            }
            Ok(p)
        }
    }

    fn command(&mut self, command: Command) {
        let gst = self.gst;
        // SAFETY: calls on the live playbin.
        unsafe {
            match command {
                Command::Playing(playing) => {
                    self.playing = playing;
                    self.prerolled = false;
                    (gst.element_set_state)(self.playbin, if playing { STATE_PLAYING } else { STATE_PAUSED });
                }
                Command::Muted(muted) => {
                    (gst.object_set)(self.playbin, c"mute".as_ptr(), c_int::from(muted), ptr::null::<c_char>())
                }
                Command::Looping(looping) => self.looping = looping,
                Command::Seek(seconds) => {
                    self.ended = !self.seek(seconds) && self.ended;
                }
                Command::Fit { width, height, cover } => {
                    self.fit = Some((width, height, cover));
                    self.ask();
                }
            }
        }
    }

    fn seek(&self, seconds: f64) -> bool {
        let ns = (seconds.max(0.) * 1e9) as i64;
        // SAFETY: a seek on the live playbin.
        unsafe { (self.gst.element_seek_simple)(self.playbin, FORMAT_TIME, SEEK_FLUSH_ACCURATE, ns) != 0 }
    }

    /// Points the caps filter at the size the box needs once the picture's own size is known.
    fn ask(&mut self) {
        let Some(native) = self.native else { return };
        let size = fit_size(native, self.fit);
        if self.asked == Some(size) {
            return;
        }
        self.asked = Some(size);
        let caps = format!("video/x-raw,format=BGRA,pixel-aspect-ratio=1/1,width={},height={}", size.0, size.1);
        let caps = CString::new(caps).expect("no NUL");
        // SAFETY: new caps handed to the filter, which takes its own reference.
        unsafe {
            let caps = (self.gst.caps_from_string)(caps.as_ptr());
            if !caps.is_null() {
                (self.gst.object_set)(self.fit_filter, c"caps".as_ptr(), caps, ptr::null::<c_char>());
                (self.gst.mini_object_unref)(caps);
            }
        }
    }

    /// The first error waiting on the bus.
    fn bus_error(&mut self) -> Option<String> {
        // SAFETY: a live bus; a popped message is ours to unref.
        unsafe {
            let message = (self.gst.bus_timed_pop_filtered)(self.bus, 0, MESSAGE_ERROR);
            if message.is_null() {
                return None;
            }
            let mut error = ptr::null_mut();
            (self.gst.message_parse_error)(message, &mut error, ptr::null_mut());
            (self.gst.mini_object_unref)(message.cast());
            Some(self.gst.take_error(error))
        }
    }

    fn bus(&mut self, shared: &Shared) -> Result<(), String> {
        let gst = self.gst;
        loop {
            // SAFETY: a live bus; a popped message is ours to unref.
            let message =
                unsafe { (gst.bus_timed_pop_filtered)(self.bus, 0, MESSAGE_EOS | MESSAGE_ERROR | MESSAGE_ASYNC_DONE) };
            if message.is_null() {
                return Ok(());
            }
            // SAFETY: a live message until the unref below.
            let kind = unsafe { (*message).type_ };
            let mut failed = None;
            match kind {
                MESSAGE_ERROR => {
                    let mut error = ptr::null_mut();
                    unsafe { (gst.message_parse_error)(message, &mut error, ptr::null_mut()) };
                    failed = Some(gst.take_error(error));
                }
                MESSAGE_EOS if self.looping => {
                    self.seek(0.);
                }
                MESSAGE_EOS => {
                    self.ended = true;
                    shared.event(Event::Ended);
                }
                MESSAGE_ASYNC_DONE => self.prerolled = true,
                _ => {}
            }
            unsafe { (gst.mini_object_unref)(message.cast()) };
            if let Some(error) = failed {
                return Err(error);
            }
        }
    }

    /// Hands over the next frame: while playing the one due now, while paused the preroll after
    /// opening or a seek.
    fn pull(&mut self, shared: &Shared) {
        let gst = self.gst;
        let sample = if self.playing && !self.ended {
            self.prerolled = false;
            unsafe { (gst.app_sink_try_pull_sample)(self.frames, PULL_TIMEOUT_NS) }
        } else if self.prerolled {
            self.prerolled = false;
            unsafe { (gst.app_sink_try_pull_preroll)(self.frames, 0) }
        } else {
            std::thread::sleep(std::time::Duration::from_nanos(PULL_TIMEOUT_NS));
            return;
        };
        if sample.is_null() {
            return;
        }
        let frame = self.read(sample);
        // SAFETY: the sample was pulled for us.
        unsafe { (gst.mini_object_unref)(sample) };
        let Some(frame) = frame else { return };
        if self.native.is_none() {
            self.native = Some((frame.width, frame.height));
            let mut ns = 0i64;
            // SAFETY: a duration query on the live playbin.
            let known = unsafe { (gst.element_query_duration)(self.playbin, FORMAT_TIME, &mut ns) } != 0;
            let duration = (known && ns > 0).then(|| ns as f64 / 1e9);
            shared.event(Event::Ready { width: frame.width, height: frame.height, duration });
            self.ask();
        }
        shared.frame(frame);
    }

    fn read(&self, sample: Obj) -> Option<Frame> {
        let gst = self.gst;
        // SAFETY: the sample is live; its buffer is mapped for reading and unmapped before return.
        unsafe {
            let caps = (gst.sample_get_caps)(sample);
            let buffer = (gst.sample_get_buffer)(sample);
            if caps.is_null() || buffer.is_null() {
                return None;
            }
            let s = (gst.caps_get_structure)(caps, 0);
            let (mut w, mut h) = (0, 0);
            if (gst.structure_get_int)(s, c"width".as_ptr(), &mut w) == 0
                || (gst.structure_get_int)(s, c"height".as_ptr(), &mut h) == 0
                || w <= 0
                || h <= 0
            {
                return None;
            }
            let mut map: GstMapInfo = std::mem::zeroed();
            if (gst.buffer_map)(buffer, &mut map, MAP_READ) == 0 {
                return None;
            }
            let len = w as usize * h as usize * 4;
            let frame = (map.size >= len).then(|| Frame {
                width: w as u32,
                height: h as u32,
                bgra: std::slice::from_raw_parts(map.data, len).to_vec(),
                position: match (*buffer).pts {
                    u64::MAX => f64::NAN,
                    pts => pts as f64 / 1e9,
                },
            });
            (gst.buffer_unmap)(buffer, &mut map);
            frame
        }
    }
}
