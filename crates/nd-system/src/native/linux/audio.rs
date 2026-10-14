//! Audio playback with a spectrum through GStreamer's `playbin`, loaded with `dlopen` at the
//! first play so a system without GStreamer still runs: play then answers "audio unavailable".
//! No GLib main loop runs in an nd app, so each pipeline's bus is popped by its own thread.

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
use std::mem::offset_of;
use std::ptr::{self, NonNull};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use libloading::Library;

use crate::backend::{AudioControl, Emitter, Reply};
use crate::spectrum::fold_db_bands;
use crate::{AudioEvent, AudioSource, AudioState, Error, PlayOptions, PlayerId, Result, SystemEvent};

const STATE_NULL: c_int = 1;
const STATE_PAUSED: c_int = 3;
const STATE_PLAYING: c_int = 4;
const STATE_CHANGE_FAILURE: c_int = 0;
const FORMAT_TIME: c_int = 3;
const SEEK_FLUSH_KEY_UNIT: c_int = 1 | 1 << 2;
const MESSAGE_EOS: c_uint = 1;
const MESSAGE_ERROR: c_uint = 1 << 1;
const MESSAGE_STATE_CHANGED: c_uint = 1 << 6;
const MESSAGE_ELEMENT: c_uint = 1 << 15;

/// 128 linear bands every 66.67 ms (about 15 Hz), silent below -60 dB.
const SPECTRUM_BANDS: c_uint = 128;
const SPECTRUM_INTERVAL_NS: u64 = 66_666_667;
const SPECTRUM_THRESHOLD_DB: c_int = -60;
/// How long a bus thread waits for a message before it checks whether its player stopped.
const POP_TIMEOUT_NS: u64 = 100_000_000;

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

/// The public head of `GstMessage`. Its type has no accessor function (`GST_MESSAGE_TYPE` is a
/// macro), so it is read from the struct.
#[repr(C)]
struct GstMessage {
    mini_object: GstMiniObject,
    type_: c_uint,
    timestamp: u64,
    src: *mut c_void,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(offset_of!(GstMessage, type_) == 64 && offset_of!(GstMessage, src) == 80);

#[repr(C)]
struct GError {
    domain: u32,
    code: c_int,
    message: *mut c_char,
}

type Obj = *mut c_void;

struct Gst {
    _libs: [Library; 3],
    element_factory_make: unsafe extern "C" fn(*const c_char, *const c_char) -> Obj,
    element_set_state: unsafe extern "C" fn(Obj, c_int) -> c_int,
    element_query_position: unsafe extern "C" fn(Obj, c_int, *mut i64) -> c_int,
    element_query_duration: unsafe extern "C" fn(Obj, c_int, *mut i64) -> c_int,
    element_seek_simple: unsafe extern "C" fn(Obj, c_int, c_int, i64) -> c_int,
    element_get_bus: unsafe extern "C" fn(Obj) -> Obj,
    element_get_static_pad: unsafe extern "C" fn(Obj, *const c_char) -> Obj,
    pad_get_current_caps: unsafe extern "C" fn(Obj) -> Obj,
    caps_get_structure: unsafe extern "C" fn(Obj, c_uint) -> Obj,
    bus_timed_pop_filtered: unsafe extern "C" fn(Obj, u64, c_uint) -> *mut GstMessage,
    mini_object_unref: unsafe extern "C" fn(Obj),
    object_ref_sink: unsafe extern "C" fn(Obj) -> Obj,
    object_unref: unsafe extern "C" fn(Obj),
    message_get_structure: unsafe extern "C" fn(*mut GstMessage) -> Obj,
    message_parse_error: unsafe extern "C" fn(*mut GstMessage, *mut *mut GError, *mut *mut c_char),
    message_parse_state_changed: unsafe extern "C" fn(*mut GstMessage, *mut c_int, *mut c_int, *mut c_int),
    structure_get_name: unsafe extern "C" fn(Obj) -> *const c_char,
    structure_get_value: unsafe extern "C" fn(Obj, *const c_char) -> Obj,
    structure_get_int: unsafe extern "C" fn(Obj, *const c_char, *mut c_int) -> c_int,
    value_list_get_size: unsafe extern "C" fn(Obj) -> c_uint,
    value_list_get_value: unsafe extern "C" fn(Obj, c_uint) -> Obj,
    filename_to_uri: unsafe extern "C" fn(*const c_char, *mut *mut GError) -> *mut c_char,
    uri_is_valid: unsafe extern "C" fn(*const c_char) -> c_int,
    object_set: unsafe extern "C" fn(Obj, *const c_char, ...),
    value_get_float: unsafe extern "C" fn(Obj) -> f32,
    error_free: unsafe extern "C" fn(*mut GError),
    free: unsafe extern "C" fn(*mut c_void),
}

fn gst() -> Result<&'static Gst> {
    static GST: OnceLock<Result<Gst, String>> = OnceLock::new();
    GST.get_or_init(Gst::load).as_ref().map_err(|e| Error::Platform(format!("audio unavailable: {e}")))
}

impl Gst {
    fn load() -> Result<Gst, String> {
        let open = |name: &str| {
            // SAFETY: these libraries' initializers have no preconditions.
            unsafe { Library::new(name) }.map_err(|e| format!("gstreamer not found ({e})"))
        };
        let gst = open("libgstreamer-1.0.so.0")?;
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
            element_set_state: sym(&gst, "gst_element_set_state")?,
            element_query_position: sym(&gst, "gst_element_query_position")?,
            element_query_duration: sym(&gst, "gst_element_query_duration")?,
            element_seek_simple: sym(&gst, "gst_element_seek_simple")?,
            element_get_bus: sym(&gst, "gst_element_get_bus")?,
            element_get_static_pad: sym(&gst, "gst_element_get_static_pad")?,
            pad_get_current_caps: sym(&gst, "gst_pad_get_current_caps")?,
            caps_get_structure: sym(&gst, "gst_caps_get_structure")?,
            bus_timed_pop_filtered: sym(&gst, "gst_bus_timed_pop_filtered")?,
            mini_object_unref: sym(&gst, "gst_mini_object_unref")?,
            object_ref_sink: sym(&gst, "gst_object_ref_sink")?,
            object_unref: sym(&gst, "gst_object_unref")?,
            message_get_structure: sym(&gst, "gst_message_get_structure")?,
            message_parse_error: sym(&gst, "gst_message_parse_error")?,
            message_parse_state_changed: sym(&gst, "gst_message_parse_state_changed")?,
            structure_get_name: sym(&gst, "gst_structure_get_name")?,
            structure_get_value: sym(&gst, "gst_structure_get_value")?,
            structure_get_int: sym(&gst, "gst_structure_get_int")?,
            value_list_get_size: sym(&gst, "gst_value_list_get_size")?,
            value_list_get_value: sym(&gst, "gst_value_list_get_value")?,
            filename_to_uri: sym(&gst, "gst_filename_to_uri")?,
            uri_is_valid: sym(&gst, "gst_uri_is_valid")?,
            object_set: sym(&gobject, "g_object_set")?,
            value_get_float: sym(&gobject, "g_value_get_float")?,
            error_free: sym(&glib, "g_error_free")?,
            free: sym(&glib, "g_free")?,
            _libs: [gst, gobject, glib],
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

    fn make(&self, factory: &CStr) -> Option<NonNull<c_void>> {
        // SAFETY: a factory name and no element name; the floating ref is sunk into ours.
        NonNull::new(unsafe { (self.element_factory_make)(factory.as_ptr(), ptr::null()) })
            .map(|e| unsafe { NonNull::new_unchecked((self.object_ref_sink)(e.as_ptr())) })
    }

    fn query(&self, element: Obj, f: unsafe extern "C" fn(Obj, c_int, *mut i64) -> c_int) -> Option<Duration> {
        let mut ns = 0i64;
        // SAFETY: `element` is a live element.
        let ok = unsafe { f(element, FORMAT_TIME, &mut ns) } != 0;
        (ok && ns >= 0).then(|| Duration::from_nanos(ns as u64))
    }
}

/// A GStreamer object pointer shared with the bus thread. GstObjects are thread safe.
#[derive(Clone, Copy)]
struct Handle(NonNull<c_void>);

// SAFETY: GstElement and GstBus are refcounted and lock internally.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Handle {
    fn ptr(self) -> Obj {
        self.0.as_ptr()
    }
}

struct Pipeline {
    id: PlayerId,
    gst: &'static Gst,
    playbin: Handle,
    spectrum: Option<Handle>,
    bus: Handle,
    events: Emitter,
    /// Held while a transition is decided and emitted, so none follows Stopped.
    track: Mutex<Track>,
}

#[derive(Default)]
struct Track {
    started: bool,
    stopped: bool,
    failed: bool,
    nyquist_hz: Option<f64>,
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        // SAFETY: the pipeline owns one reference to each object.
        unsafe {
            (self.gst.element_set_state)(self.playbin.ptr(), STATE_NULL);
            if let Some(spectrum) = self.spectrum {
                (self.gst.object_unref)(spectrum.ptr());
            }
            (self.gst.object_unref)(self.bus.ptr());
            (self.gst.object_unref)(self.playbin.ptr());
        }
    }
}

impl Pipeline {
    fn emit(&self, event: AudioEvent) {
        self.events.emit(SystemEvent::Audio { player: self.id, event });
    }

    fn state(&self, state: AudioState, error: Option<String>) -> AudioEvent {
        let position = self.gst.query(self.playbin.ptr(), self.gst.element_query_position).unwrap_or_default();
        let duration = self.gst.query(self.playbin.ptr(), self.gst.element_query_duration);
        AudioEvent::State { state, position, duration, error }
    }

    fn set_state(&self, state: c_int) -> c_int {
        // SAFETY: a live element.
        unsafe { (self.gst.element_set_state)(self.playbin.ptr(), state) }
    }

    fn set_volume(&self, volume: f32) {
        // SAFETY: playbin's `volume` is a gdouble; the list ends with null.
        unsafe {
            (self.gst.object_set)(
                self.playbin.ptr(),
                c"volume".as_ptr(),
                f64::from(volume.clamp(0., 1.)),
                ptr::null::<c_char>(),
            )
        };
    }

    fn fail(&self, track: &mut Track, message: String) {
        if !track.failed {
            track.failed = true;
            self.emit(self.state(AudioState::Error, Some(message)));
        }
    }

    fn watch(self: Arc<Self>) {
        let gst = self.gst;
        let mask = MESSAGE_EOS | MESSAGE_ERROR | MESSAGE_STATE_CHANGED | MESSAGE_ELEMENT;
        loop {
            // SAFETY: a live bus; a popped message is ours to unref.
            let message = unsafe { (gst.bus_timed_pop_filtered)(self.bus.ptr(), POP_TIMEOUT_NS, mask) };
            let mut track = self.track.lock().expect("audio track");
            if !message.is_null() {
                if !track.stopped {
                    self.handle(&mut track, message);
                }
                unsafe { (gst.mini_object_unref)(message.cast()) };
            }
            if track.stopped {
                return;
            }
        }
    }

    fn handle(&self, track: &mut Track, message: *mut GstMessage) {
        let gst = self.gst;
        // SAFETY: `message` is a live GstMessage for the duration of this call.
        let (kind, src) = unsafe { ((*message).type_, (*message).src) };
        match kind {
            MESSAGE_EOS => {
                let mut event = self.state(AudioState::Ended, None);
                if let AudioEvent::State { position, duration: Some(duration), .. } = &mut event {
                    *position = *duration;
                }
                self.emit(event);
            }
            MESSAGE_ERROR => {
                let mut error = ptr::null_mut();
                unsafe { (gst.message_parse_error)(message, &mut error, ptr::null_mut()) };
                let text = gst.take_error(error);
                self.set_state(STATE_NULL);
                self.fail(track, text);
            }
            MESSAGE_STATE_CHANGED if src == self.playbin.ptr() => {
                let (mut old, mut new, mut pending) = (0, 0, 0);
                unsafe { (gst.message_parse_state_changed)(message, &mut old, &mut new, &mut pending) };
                if new == STATE_PLAYING && !track.started {
                    track.started = true;
                    self.emit(self.state(AudioState::Playing, None));
                }
            }
            MESSAGE_ELEMENT if self.spectrum.is_some_and(|s| s.ptr() == src) => {
                if let Some(bands) = self.magnitudes(message) {
                    let nyquist = *track.nyquist_hz.get_or_insert_with(|| self.nyquist_hz());
                    self.emit(AudioEvent::Spectrum(fold_db_bands(&bands, nyquist).to_vec()));
                }
            }
            _ => {}
        }
    }

    fn magnitudes(&self, message: *mut GstMessage) -> Option<Vec<f32>> {
        let gst = self.gst;
        // SAFETY: structure and values are borrowed from the live message.
        unsafe {
            let s = (gst.message_get_structure)(message);
            if s.is_null() || CStr::from_ptr((gst.structure_get_name)(s)) != c"spectrum" {
                return None;
            }
            let list = (gst.structure_get_value)(s, c"magnitude".as_ptr());
            if list.is_null() {
                return None;
            }
            let n = (gst.value_list_get_size)(list);
            Some((0..n).map(|i| (gst.value_get_float)((gst.value_list_get_value)(list, i))).collect())
        }
    }

    /// Half the rate the spectrum element negotiated, 22.05 kHz when it cannot be read.
    fn nyquist_hz(&self) -> f64 {
        let gst = self.gst;
        let Some(spectrum) = self.spectrum else { return 22_050. };
        let mut rate: c_int = 0;
        // SAFETY: pad and caps are new references, released here.
        unsafe {
            let pad = (gst.element_get_static_pad)(spectrum.ptr(), c"sink".as_ptr());
            if !pad.is_null() {
                let caps = (gst.pad_get_current_caps)(pad);
                if !caps.is_null() {
                    let s = (gst.caps_get_structure)(caps, 0);
                    if !s.is_null() {
                        (gst.structure_get_int)(s, c"rate".as_ptr(), &mut rate);
                    }
                    (gst.mini_object_unref)(caps);
                }
                (gst.object_unref)(pad);
            }
        }
        if rate > 0 { f64::from(rate) / 2. } else { 22_050. }
    }
}

pub struct Audio {
    events: Mutex<Emitter>,
    players: Mutex<HashMap<PlayerId, Arc<Pipeline>>>,
}

impl Audio {
    pub fn new() -> Self {
        Self { events: Mutex::new(Emitter::detached()), players: Mutex::default() }
    }

    pub fn set_emitter(&self, events: Emitter) {
        *self.events.lock().expect("audio emitter") = events;
    }

    pub fn play(&self, id: PlayerId, options: PlayOptions, reply: Reply<()>) {
        reply.send(self.start(id, options));
    }

    pub fn control(&self, id: PlayerId, control: AudioControl, reply: Reply<()>) {
        reply.send(self.apply(id, control));
    }

    fn start(&self, id: PlayerId, options: PlayOptions) -> Result<()> {
        let gst = gst()?;
        let uri = uri(gst, &options.source)?;
        let playbin =
            gst.make(c"playbin").ok_or_else(|| Error::Platform("audio unavailable: no playbin element".into()))?;
        // SAFETY: `uri` is a gchararray property; the list ends with null.
        unsafe { (gst.object_set)(playbin.as_ptr(), c"uri".as_ptr(), uri.as_ptr(), ptr::null::<c_char>()) };
        let spectrum = if options.spectrum { gst.make(c"spectrum") } else { None };
        if let Some(spectrum) = spectrum {
            // SAFETY: each value has its property's C type; playbin takes its own reference to
            // the filter.
            unsafe {
                (gst.object_set)(
                    spectrum.as_ptr(),
                    c"bands".as_ptr(),
                    SPECTRUM_BANDS,
                    c"interval".as_ptr(),
                    SPECTRUM_INTERVAL_NS,
                    c"threshold".as_ptr(),
                    SPECTRUM_THRESHOLD_DB,
                    c"post-messages".as_ptr(),
                    1 as c_int,
                    ptr::null::<c_char>(),
                );
                (gst.object_set)(playbin.as_ptr(), c"audio-filter".as_ptr(), spectrum.as_ptr(), ptr::null::<c_char>());
            }
        }
        // SAFETY: a pipeline always has a bus; this is a new reference.
        let bus = NonNull::new(unsafe { (gst.element_get_bus)(playbin.as_ptr()) }).expect("playbin has a bus");
        let pipeline = Arc::new(Pipeline {
            id,
            gst,
            playbin: Handle(playbin),
            spectrum: spectrum.map(Handle),
            bus: Handle(bus),
            events: self.events.lock().expect("audio emitter").clone(),
            track: Mutex::default(),
        });
        pipeline.set_volume(options.volume);

        let watcher = pipeline.clone();
        std::thread::Builder::new()
            .name(format!("nd-audio-{}", id.0))
            .spawn(move || watcher.watch())
            .map_err(Error::platform)?;
        self.players.lock().expect("audio players").insert(id, pipeline.clone());
        if pipeline.set_state(STATE_PLAYING) == STATE_CHANGE_FAILURE {
            let mut track = pipeline.track.lock().expect("audio track");
            pipeline.fail(&mut track, "playback failed to start".into());
        }
        Ok(())
    }

    fn apply(&self, id: PlayerId, control: AudioControl) -> Result<()> {
        let pipeline = if control == AudioControl::Stop {
            self.players.lock().expect("audio players").remove(&id)
        } else {
            self.players.lock().expect("audio players").get(&id).cloned()
        }
        .ok_or(Error::NotFound)?;
        let mut track = pipeline.track.lock().expect("audio track");
        match control {
            AudioControl::Pause => {
                pipeline.set_state(STATE_PAUSED);
                track.started = true;
                pipeline.emit(pipeline.state(AudioState::Paused, None));
            }
            AudioControl::Resume => {
                pipeline.set_state(STATE_PLAYING);
                track.started = true;
                pipeline.emit(pipeline.state(AudioState::Playing, None));
            }
            AudioControl::Stop => {
                let event = pipeline.state(AudioState::Stopped, None);
                pipeline.set_state(STATE_NULL);
                track.stopped = true;
                pipeline.emit(event);
            }
            AudioControl::Seek(to) => {
                let ns = i64::try_from(to.as_nanos()).unwrap_or(i64::MAX);
                // SAFETY: a live element.
                let ok = unsafe {
                    (pipeline.gst.element_seek_simple)(pipeline.playbin.ptr(), FORMAT_TIME, SEEK_FLUSH_KEY_UNIT, ns)
                };
                if ok == 0 {
                    return Err(Error::Platform(format!("audio: cannot seek to {to:?}")));
                }
            }
            AudioControl::SetVolume(volume) => pipeline.set_volume(volume),
        }
        Ok(())
    }
}

fn uri(gst: &Gst, source: &AudioSource) -> Result<CString> {
    let invalid = |what: String| Error::Invalid(what);
    match source {
        AudioSource::Path(path) => {
            if !path.exists() {
                return Err(invalid(format!("audio file not found: {}", path.display())));
            }
            let path = std::path::absolute(path).map_err(Error::platform)?;
            let c = CString::new(path.as_os_str().as_encoded_bytes())
                .map_err(|_| invalid(format!("bad path: {}", path.display())))?;
            let mut error = ptr::null_mut();
            // SAFETY: a NUL-terminated path; the result is a g_malloc'd string.
            let raw = unsafe { (gst.filename_to_uri)(c.as_ptr(), &mut error) };
            if raw.is_null() {
                return Err(Error::Platform(gst.take_error(error)));
            }
            let uri = unsafe { CStr::from_ptr(raw) }.to_owned();
            unsafe { (gst.free)(raw.cast()) };
            Ok(uri)
        }
        AudioSource::Url(url) => {
            let c = CString::new(url.as_str()).map_err(|_| invalid(format!("invalid audio url: {url}")))?;
            // SAFETY: a NUL-terminated string.
            if unsafe { (gst.uri_is_valid)(c.as_ptr()) } == 0 {
                return Err(invalid(format!("invalid audio url: {url}")));
            }
            Ok(c)
        }
    }
}
