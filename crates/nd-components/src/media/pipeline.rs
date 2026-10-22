//! The image pipeline both backends draw `image()` from: fetch, decode off the main thread at
//! the size the image is shown, keep decoded frames in a memory LRU and downloads in a disk
//! cache. Backends never decode; they ask for a [`DecodeRequest`] and get RGBA frames back.
//!
//! - Display-size decode: the box is rounded up to a multiple of [`EDGE_STEP`] device pixels,
//!   so a resize of a few points reuses the decode, and an image is never scaled up.
//! - Header probe: [`ImagePipeline::intrinsic_size`] reads only the file header, so a row can
//!   be laid out at its final size before anything decodes. For a URL that is not on disk yet
//!   the app passes the size it knows (`image().size_hint(..)`).
//! - Animated GIF and WebP decode every frame at the display size. An animation over
//!   [`PipelineConfig::animation_budget`] drops every other frame, folding its delay into the
//!   kept one, until it fits; a frame delay under 20 ms plays at 100 ms, as browsers do.
//! - Memory: decoded bytes are capped at [`PipelineConfig::memory_budget`], least recently used
//!   first, except entries used in the last [`IN_USE`] and the newest one, so what is on screen
//!   never flickers back to its placeholder.
//! - Work runs on [`PipelineConfig::workers`] threads, newest request first: when a list
//!   scrolls fast, what the user stopped on decodes before what already scrolled away. Requests
//!   for the same key share one decode. A failure is remembered for [`RETRY_AFTER`].
//! - Downloads land in `<cache dir>/images/<FNV-1a of the URL>` through a temp file and a
//!   rename, so a crash never leaves half a file under the final name.
//! - SVG renders with resvg at the box size (a vector scales up too); `data:` URLs (base64 or
//!   percent-encoded) are bytes. A request with a tint fills every pixel with the tint's colour
//!   and keeps the pixel's alpha, for monochrome icons in a token colour.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbaImage};
use nd_core::time::{Duration, Instant};
use nd_ui::prelude::SharedString;

/// Display-size decodes round the box up to a multiple of this many device pixels.
pub const EDGE_STEP: u32 = 64;
/// An entry drawn this recently is never evicted.
pub const IN_USE: Duration = Duration::from_secs(1);
/// A failed load is not retried before this.
pub const RETRY_AFTER: Duration = Duration::from_secs(20);
/// The shortest frame delay played as given; shorter ones play at [`SLOW_FRAME`].
pub const MIN_FRAME: Duration = Duration::from_millis(20);
pub const SLOW_FRAME: Duration = Duration::from_millis(100);

/// Where an image comes from. Equal sources share decodes and cache entries.
#[derive(Clone)]
pub enum ImageSource {
    Path(Arc<Path>),
    /// `https:` or `http:`; downloaded once into the disk cache.
    Url(SharedString),
    /// Bytes the app already holds (a pasted image, a thumbnail from a server). `key` names
    /// them in the caches, so two different images must not share one.
    Bytes {
        key: SharedString,
        bytes: Arc<[u8]>,
    },
}

impl ImageSource {
    pub fn path(path: impl AsRef<Path>) -> Self {
        ImageSource::Path(Arc::from(path.as_ref()))
    }

    pub fn url(url: impl Into<SharedString>) -> Self {
        ImageSource::Url(url.into())
    }

    pub fn bytes(key: impl Into<SharedString>, bytes: impl Into<Arc<[u8]>>) -> Self {
        ImageSource::Bytes { key: key.into(), bytes: bytes.into() }
    }

    /// What the caches key it by.
    pub fn key(&self) -> String {
        match self {
            ImageSource::Path(p) => format!("file:{}", p.display()),
            ImageSource::Url(u) => u.to_string(),
            ImageSource::Bytes { key, .. } => format!("bytes:{key}"),
        }
    }
}

impl PartialEq for ImageSource {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ImageSource::Path(a), ImageSource::Path(b)) => a == b,
            (ImageSource::Url(a), ImageSource::Url(b)) => a == b,
            (ImageSource::Bytes { key: a, .. }, ImageSource::Bytes { key: b, .. }) => a == b,
            _ => false,
        }
    }
}

impl Eq for ImageSource {}

impl Hash for ImageSource {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

impl fmt::Debug for ImageSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageSource::Bytes { key, bytes } => write!(f, "Bytes({key}, {} bytes)", bytes.len()),
            other => f.write_str(&other.key()),
        }
    }
}

/// The bytes a `data:` URL carries: base64 when it says so, else percent-encoded text.
pub fn data_url_bytes(url: &str) -> Option<Vec<u8>> {
    let (head, body) = url.strip_prefix("data:")?.split_once(',')?;
    if head.ends_with(";base64") {
        use base64::Engine;
        let body: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        return base64::engine::general_purpose::STANDARD.decode(body).ok();
    }
    let bytes = body.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(a), Some(b)) => {
                out.push((a * 16 + b) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    Some(out)
}

impl From<&str> for ImageSource {
    /// A string with a scheme is a URL, a `data:` URL its bytes, anything else a path.
    fn from(s: &str) -> Self {
        if s.starts_with("data:") {
            let bytes = data_url_bytes(s).unwrap_or_default();
            return ImageSource::bytes(format!("data:{:016x}", fnv1a(s)), bytes);
        }
        if s.starts_with("https:") || s.starts_with("http:") {
            ImageSource::url(SharedString::from(s.to_owned()))
        } else {
            ImageSource::path(s.strip_prefix("file://").unwrap_or(s))
        }
    }
}

impl From<String> for ImageSource {
    fn from(s: String) -> Self {
        ImageSource::from(s.as_str())
    }
}

impl From<PathBuf> for ImageSource {
    fn from(p: PathBuf) -> Self {
        ImageSource::Path(Arc::from(p.as_path()))
    }
}

/// How an image fills a box of another aspect ratio.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ImageFit {
    /// Fills the box, cropping the overflow around the centre (artwork, avatars).
    #[default]
    Cover,
    /// Fits inside the box, letterboxed (photos shown whole).
    Contain,
}

/// One decode: a source at a box size in device pixels.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DecodeRequest {
    pub source: ImageSource,
    /// The box in device pixels, each side rounded up to [`EDGE_STEP`]; 0 decodes full size.
    pub width: u32,
    pub height: u32,
    pub fit: ImageFit,
    /// Straight sRGB RGBA every pixel takes, keeping its own alpha times the tint's.
    pub tint: Option<[u8; 4]>,
    /// A gaussian blur's sigma in tenths of a decoded pixel, edges held; 0 for none.
    pub blur: u32,
}

impl DecodeRequest {
    /// The request for showing `source` in a `width` x `height` point box at `scale`.
    pub fn at(source: ImageSource, width: f32, height: f32, scale: f32, fit: ImageFit) -> Self {
        // Below one pixel per point only for a blurred picture, which wants its few pixels as
        // they are: rounding up to the step would sharpen it again.
        let scale = if scale > 0. { scale } else { 1. };
        let step = |v: f32| -> u32 {
            let px = (v.max(0.) * scale).ceil() as u32;
            match px {
                0 => 0,
                _ if scale < 1. => px,
                _ => px.div_ceil(EDGE_STEP) * EDGE_STEP,
            }
        };
        Self { source, width: step(width), height: step(height), fit, tint: None, blur: 0 }
    }

    /// The same request blurred by `sigma` decoded pixels.
    pub fn blurred(mut self, sigma: f32) -> Self {
        self.blur = (sigma.max(0.) * 10.).round() as u32;
        self
    }

    /// The same request drawn in `tint` (a theme token).
    pub fn tinted(mut self, tint: Option<nd_ui::prelude::Hsla>) -> Self {
        self.tint = tint.map(|c| {
            let c = c.to_rgb();
            [c.r, c.g, c.b, c.a].map(|v| (v.clamp(0., 1.) * 255.).round() as u8)
        });
        self
    }
}

/// One decoded frame, straight (not premultiplied) RGBA, row-major.
#[derive(Clone)]
pub struct Frame {
    pub rgba: Arc<[u8]>,
    pub delay: Duration,
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Frame({} bytes, {:?})", self.rgba.len(), self.delay)
    }
}

/// A decoded image: one frame for a still, several for an animation.
#[derive(Clone, Debug)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    /// The file's own size, before the display-size decode.
    pub source_width: u32,
    pub source_height: u32,
    pub frames: Vec<Frame>,
}

impl Decoded {
    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }

    /// Decoded bytes, what the memory budget counts.
    pub fn bytes(&self) -> usize {
        self.frames.iter().map(|f| f.rgba.len()).sum()
    }

    /// One loop of the animation.
    pub fn loop_duration(&self) -> Duration {
        self.frames.iter().map(|f| f.delay).sum()
    }

    /// The frame showing `elapsed` into playback, looping.
    pub fn frame_at(&self, elapsed: Duration) -> usize {
        let total = self.loop_duration();
        if self.frames.len() < 2 || total.is_zero() {
            return 0;
        }
        let mut t = Duration::from_nanos((elapsed.as_nanos() % total.as_nanos()) as u64);
        for (i, f) in self.frames.iter().enumerate() {
            if t < f.delay {
                return i;
            }
            t -= f.delay;
        }
        self.frames.len() - 1
    }
}

pub type LoadResult = Result<Arc<Decoded>, String>;
type Done = Box<dyn FnOnce(LoadResult) + Send>;
/// Fetches a URL's bytes; tests swap in their own.
pub type Fetch = Arc<dyn Fn(&str) -> Result<Vec<u8>, String> + Send + Sync>;

#[derive(Clone)]
pub struct PipelineConfig {
    /// Decoded bytes kept in memory.
    pub memory_budget: usize,
    /// Largest decoded size of one animation before frames are thinned.
    pub animation_budget: usize,
    /// Download cache; `None` keeps downloads in memory only.
    pub disk_dir: Option<PathBuf>,
    pub workers: usize,
    pub fetch: Fetch,
}

impl fmt::Debug for PipelineConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PipelineConfig")
            .field("memory_budget", &self.memory_budget)
            .field("animation_budget", &self.animation_budget)
            .field("disk_dir", &self.disk_dir)
            .field("workers", &self.workers)
            .finish_non_exhaustive()
    }
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            memory_budget: 64 << 20,
            animation_budget: 48 << 20,
            disk_dir: Some(nd_core::paths::app_cache_dir(&app_name()).join("images")),
            workers: 3,
            fetch: Arc::new(http_get),
        }
    }
}

fn app_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "nd".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn http_get(url: &str) -> Result<Vec<u8>, String> {
    let mut response = ureq::get(url).call().map_err(|e| e.to_string())?;
    response.body_mut().with_config().limit(64 << 20).read_to_vec().map_err(|e| e.to_string())
}

#[cfg(target_arch = "wasm32")]
fn http_get(url: &str) -> Result<Vec<u8>, String> {
    Err(format!("{url}: the web backend fetches asynchronously, through the pipeline's queue"))
}

/// `fetch(url)` in the page, resolved against the page's own URL.
#[cfg(target_arch = "wasm32")]
async fn web_fetch(url: &str) -> Result<Vec<u8>, String> {
    use wasm_bindgen::JsCast;
    let window = web_sys::window().ok_or("no window")?;
    let response =
        wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(url)).await.map_err(|e| format!("{url}: {e:?}"))?;
    let response: web_sys::Response = response.dyn_into().map_err(|_| format!("{url}: not a Response"))?;
    if !response.ok() {
        return Err(format!("{url}: HTTP {}", response.status()));
    }
    let buffer = response.array_buffer().map_err(|e| format!("{url}: {e:?}"))?;
    let buffer = wasm_bindgen_futures::JsFuture::from(buffer).await.map_err(|e| format!("{url}: {e:?}"))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

enum Slot {
    Ready { image: Arc<Decoded>, used: Instant, inserted: u64 },
    Failed { error: String, at: Instant },
}

struct Job {
    request: DecodeRequest,
}

#[derive(Default)]
struct State {
    cache: HashMap<DecodeRequest, Slot>,
    bytes: usize,
    inserted: u64,
    waiting: HashMap<DecodeRequest, Vec<Done>>,
    queue: VecDeque<Job>,
    sizes: HashMap<ImageSource, Option<(u32, u32)>>,
    #[cfg(not(target_arch = "wasm32"))]
    threads: usize,
}

struct Shared {
    config: PipelineConfig,
    state: Mutex<State>,
    work: Condvar,
}

/// The decode queue and caches. Cheap to clone; [`ImagePipeline::global`] is the one apps use.
#[derive(Clone)]
pub struct ImagePipeline(Arc<Shared>);

impl ImagePipeline {
    pub fn new(config: PipelineConfig) -> Self {
        Self(Arc::new(Shared { config, state: Mutex::default(), work: Condvar::new() }))
    }

    /// The process's pipeline, with [`PipelineConfig::default`].
    pub fn global() -> &'static ImagePipeline {
        static GLOBAL: OnceLock<ImagePipeline> = OnceLock::new();
        GLOBAL.get_or_init(|| ImagePipeline::new(PipelineConfig::default()))
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The decoded image for `request` when it is in memory; marks it in use.
    pub fn get(&self, request: &DecodeRequest) -> Option<Arc<Decoded>> {
        let mut state = self.state();
        match state.cache.get_mut(request) {
            Some(Slot::Ready { image, used, .. }) => {
                *used = Instant::now();
                Some(image.clone())
            }
            _ => None,
        }
    }

    /// The error of a failed load still inside its retry wait.
    pub fn failed(&self, request: &DecodeRequest) -> Option<String> {
        match self.state().cache.get(request) {
            Some(Slot::Failed { error, at }) if at.elapsed() < RETRY_AFTER => Some(error.clone()),
            _ => None,
        }
    }

    /// Marks a shown image as in use, so eviction passes it over.
    pub fn touch(&self, request: &DecodeRequest) {
        if let Some(Slot::Ready { used, .. }) = self.state().cache.get_mut(request) {
            *used = Instant::now();
        }
    }

    /// Decodes `request` off the calling thread and calls `done` with the result, on a worker
    /// thread. An image already in memory, or a failure inside its retry wait, calls `done`
    /// before returning.
    pub fn load(&self, request: DecodeRequest, done: impl FnOnce(LoadResult) + Send + 'static) {
        let mut state = self.state();
        match state.cache.get_mut(&request) {
            Some(Slot::Ready { image, used, .. }) => {
                *used = Instant::now();
                let image = image.clone();
                drop(state);
                done(Ok(image));
                return;
            }
            Some(Slot::Failed { error, at }) if at.elapsed() < RETRY_AFTER => {
                let error = error.clone();
                drop(state);
                done(Err(error));
                return;
            }
            _ => {}
        }
        let waiters = state.waiting.entry(request.clone()).or_default();
        waiters.push(Box::new(done));
        if waiters.len() > 1 {
            // Already queued or decoding: it goes to the front again, it is wanted now.
            if let Some(i) = state.queue.iter().position(|j| j.request == request) {
                let job = state.queue.remove(i).expect("queued job");
                state.queue.push_front(job);
            }
            return;
        }
        state.queue.push_front(Job { request });
        self.spawn_workers(&mut state);
        drop(state);
        self.0.work.notify_one();
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn spawn_workers(&self, state: &mut State) {
        while state.threads < self.0.config.workers.max(1) {
            state.threads += 1;
            let shared = Arc::downgrade(&self.0);
            let name = format!("nd-image-{}", state.threads);
            let spawned = std::thread::Builder::new().name(name).spawn(move || worker(shared));
            if spawned.is_err() {
                state.threads -= 1;
                break;
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn spawn_workers(&self, state: &mut State) {
        // No threads on the web: bytes decode now, on the caller's turn. A URL, or a path (which
        // on the web is relative to the page, e.g. assets the bundle ships), is fetched first.
        while let Some(job) = state.queue.pop_front() {
            let url = match &job.request.source {
                ImageSource::Bytes { .. } => None,
                ImageSource::Url(url) => Some(url.to_string()),
                ImageSource::Path(path) => Some(path.to_string_lossy().into_owned()),
            };
            let Some(url) = url else {
                let result = run(&self.0.config, &job.request);
                let waiters = state.waiting.remove(&job.request).unwrap_or_default();
                finish(&self.0.config, state, &job.request, &result);
                for done in waiters {
                    done(result.clone());
                }
                continue;
            };
            let shared = self.0.clone();
            let request = job.request;
            wasm_bindgen_futures::spawn_local(async move {
                let result = match web_fetch(&url).await {
                    Ok(bytes) => decode(&bytes, &request, shared.config.animation_budget).map(Arc::new),
                    Err(error) => Err(error),
                };
                let waiters = {
                    let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                    finish(&shared.config, &mut state, &request, &result);
                    state.waiting.remove(&request).unwrap_or_default()
                };
                for done in waiters {
                    done(result.clone());
                }
            });
        }
    }

    /// The image's own size from its header, without decoding: synchronous for files, bytes
    /// and URLs already on disk, `None` for a URL not downloaded yet or a header that does not
    /// parse. Cached per source.
    pub fn intrinsic_size(&self, source: &ImageSource) -> Option<(u32, u32)> {
        if let Some(found) = self.state().sizes.get(source) {
            return *found;
        }
        let size = match source {
            // On the web a path is fetched like a URL: unknown until it has loaded.
            ImageSource::Path(_) if cfg!(target_arch = "wasm32") => return None,
            ImageSource::Path(p) => header_size_file(p),
            ImageSource::Bytes { bytes, .. } => header_size_bytes(bytes),
            ImageSource::Url(url) => {
                let path = self.0.config.disk_dir.as_ref().map(|d| d.join(disk_name(url)));
                match path {
                    Some(p) if p.exists() => header_size_file(&p),
                    // Not cached yet: unknown, and not remembered, so the next ask after the
                    // download answers.
                    _ => return None,
                }
            }
        };
        self.state().sizes.insert(source.clone(), size);
        size
    }

    /// Decoded bytes held in memory.
    pub fn memory_used(&self) -> usize {
        self.state().bytes
    }

    /// Requests waiting for a worker.
    pub fn queued(&self) -> usize {
        self.state().queue.len()
    }

    pub fn config(&self) -> &PipelineConfig {
        &self.0.config
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn worker(shared: std::sync::Weak<Shared>) {
    loop {
        let Some(shared) = shared.upgrade() else { return };
        let job = {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(job) = state.queue.pop_front() {
                    break job;
                }
                let (next, timeout) = shared
                    .work
                    .wait_timeout(state, std::time::Duration::from_secs(30))
                    .unwrap_or_else(|e| e.into_inner());
                state = next;
                // An idle worker exits; the next load starts it again.
                if timeout.timed_out() && state.queue.is_empty() {
                    state.threads -= 1;
                    return;
                }
            }
        };
        let result = run(&shared.config, &job.request);
        let waiters = {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            finish(&shared.config, &mut state, &job.request, &result);
            state.waiting.remove(&job.request).unwrap_or_default()
        };
        for done in waiters {
            done(result.clone());
        }
    }
}

/// Records `result` in the cache and evicts over budget.
fn finish(config: &PipelineConfig, state: &mut State, request: &DecodeRequest, result: &LoadResult) {
    let now = Instant::now();
    match result {
        Ok(image) => {
            state.inserted += 1;
            state.bytes += image.bytes();
            let slot = Slot::Ready { image: image.clone(), used: now, inserted: state.inserted };
            if let Some(Slot::Ready { image: old, .. }) = state.cache.insert(request.clone(), slot) {
                state.bytes -= old.bytes();
            }
            // A URL's size is known now that it is on disk.
            state.sizes.entry(request.source.clone()).or_insert(Some((image.source_width, image.source_height)));
            evict(config.memory_budget, state, now);
        }
        Err(error) => {
            state.cache.insert(request.clone(), Slot::Failed { error: error.clone(), at: now });
        }
    }
}

fn evict(budget: usize, state: &mut State, now: Instant) {
    if state.bytes <= budget {
        return;
    }
    let newest = state.inserted;
    let mut candidates: Vec<(Instant, DecodeRequest, usize)> = state
        .cache
        .iter()
        .filter_map(|(k, slot)| match slot {
            Slot::Ready { image, used, inserted } if *inserted != newest && now.duration_since(*used) >= IN_USE => {
                Some((*used, k.clone(), image.bytes()))
            }
            _ => None,
        })
        .collect();
    candidates.sort_by_key(|c| c.0);
    for (_, key, bytes) in candidates {
        if state.bytes <= budget {
            break;
        }
        state.cache.remove(&key);
        state.bytes -= bytes;
    }
}

/// FNV-1a, 64 bit: stable across runs and platforms, unlike `DefaultHasher`.
pub fn fnv1a(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn disk_name(url: &str) -> String {
    format!("{:016x}", fnv1a(url))
}

fn header_size_file(path: &Path) -> Option<(u32, u32)> {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
        return header_size_bytes(&std::fs::read(path).ok()?);
    }
    let reader = ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    oriented(reader.into_dimensions().ok()?, orientation_of_file(path))
}

fn header_size_bytes(bytes: &[u8]) -> Option<(u32, u32)> {
    if is_svg(bytes) {
        let size = svg_tree(bytes).ok()?.size();
        return Some((size.width().ceil() as u32, size.height().ceil() as u32));
    }
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
    oriented(reader.into_dimensions().ok()?, orientation_of_bytes(bytes))
}

/// Sizes as shown: a JPEG rotated a quarter turn by its EXIF orientation swaps sides.
fn oriented((w, h): (u32, u32), orientation: Option<image::metadata::Orientation>) -> Option<(u32, u32)> {
    use image::metadata::Orientation::*;
    match orientation {
        Some(Rotate90 | Rotate270 | Rotate90FlipH | Rotate270FlipH) => Some((h, w)),
        _ => Some((w, h)),
    }
}

fn orientation_of_file(path: &Path) -> Option<image::metadata::Orientation> {
    let mut decoder = ImageReader::open(path).ok()?.with_guessed_format().ok()?.into_decoder().ok()?;
    decoder.orientation().ok()
}

fn orientation_of_bytes(bytes: &[u8]) -> Option<image::metadata::Orientation> {
    let mut decoder = ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?.into_decoder().ok()?;
    decoder.orientation().ok()
}

/// Reads the source's bytes, downloading a URL into the disk cache first.
fn bytes_of(config: &PipelineConfig, source: &ImageSource) -> Result<Arc<[u8]>, String> {
    match source {
        ImageSource::Path(p) => std::fs::read(p).map(Arc::from).map_err(|e| format!("{}: {e}", p.display())),
        ImageSource::Bytes { bytes, .. } => Ok(bytes.clone()),
        ImageSource::Url(url) => {
            let Some(dir) = &config.disk_dir else { return (config.fetch)(url).map(Arc::from) };
            let path = dir.join(disk_name(url));
            if let Ok(bytes) = std::fs::read(&path) {
                return Ok(Arc::from(bytes));
            }
            let bytes = (config.fetch)(url)?;
            if std::fs::create_dir_all(dir).is_ok() {
                let tmp = dir.join(format!("{}.{}.tmp", disk_name(url), std::process::id()));
                if std::fs::write(&tmp, &bytes).is_ok() && std::fs::rename(&tmp, &path).is_err() {
                    let _ = std::fs::remove_file(&tmp);
                }
            }
            Ok(Arc::from(bytes))
        }
    }
}

fn run(config: &PipelineConfig, request: &DecodeRequest) -> LoadResult {
    let bytes = bytes_of(config, &request.source)?;
    decode(&bytes, request, config.animation_budget).map(Arc::new)
}

/// The scale that shows a `w` x `h` image in the request's box, never above 1.
fn scale_for(w: u32, h: u32, request: &DecodeRequest) -> f32 {
    if request.width == 0 || request.height == 0 || w == 0 || h == 0 {
        return 1.;
    }
    let sx = request.width as f32 / w as f32;
    let sy = request.height as f32 / h as f32;
    let s = match request.fit {
        ImageFit::Cover => sx.max(sy),
        ImageFit::Contain => sx.min(sy),
    };
    s.min(1.)
}

fn resize(image: RgbaImage, scale: f32) -> RgbaImage {
    if scale >= 1. {
        return image;
    }
    let (w, h) = image.dimensions();
    let nw = ((w as f32 * scale).round() as u32).max(1);
    let nh = ((h as f32 * scale).round() as u32).max(1);
    // Triangle is sharp enough for thumbnails and several times faster than CatmullRom; small
    // targets (avatars) gain nothing from a wider kernel.
    let filter = if nw.max(nh) <= 24 { FilterType::Triangle } else { FilterType::CatmullRom };
    image::imageops::resize(&image, nw, nh, filter)
}

/// SVG text, possibly after a BOM, an XML declaration or comments.
fn is_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(1024)];
    let text = String::from_utf8_lossy(head);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    (text.starts_with("<svg") || text.starts_with("<?xml") || text.starts_with("<!--")) && text.contains("<svg")
        || text.starts_with("<svg")
}

fn svg_tree(bytes: &[u8]) -> Result<resvg::usvg::Tree, String> {
    resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).map_err(|e| e.to_string())
}

/// Renders an SVG to fill the request's box by its fit, at its own size for an open box.
fn decode_svg(bytes: &[u8], request: &DecodeRequest) -> Result<Decoded, String> {
    let tree = svg_tree(bytes)?;
    let size = tree.size();
    let (sw, sh) = (size.width(), size.height());
    let scale = if request.width == 0 || request.height == 0 {
        1.
    } else {
        let (sx, sy) = (request.width as f32 / sw, request.height as f32 / sh);
        match request.fit {
            ImageFit::Cover => sx.max(sy),
            ImageFit::Contain => sx.min(sy),
        }
    };
    let (w, h) = (((sw * scale).round() as u32).max(1), ((sh * scale).round() as u32).max(1));
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or("svg too large to render")?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    // tiny-skia keeps premultiplied alpha; frames are straight.
    let rgba: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Ok(Decoded {
        width: w,
        height: h,
        source_width: sw.ceil() as u32,
        source_height: sh.ceil() as u32,
        frames: vec![Frame { rgba: Arc::from(rgba), delay: Duration::ZERO }],
    })
}

/// Every pixel in `tint`'s colour, its alpha scaled by the tint's.
fn apply_tint(decoded: &mut Decoded, [r, g, b, a]: [u8; 4]) {
    for frame in &mut decoded.frames {
        let mut rgba = frame.rgba.to_vec();
        for px in rgba.chunks_exact_mut(4) {
            px[3] = ((u16::from(px[3]) * u16::from(a) + 127) / 255) as u8;
            px[..3].copy_from_slice(&[r, g, b]);
        }
        frame.rgba = Arc::from(rgba);
    }
}

/// A gaussian blur over every frame, each edge pixel standing in for what lies past it, so the
/// picture keeps its own colour to its edges.
fn apply_blur(decoded: &mut Decoded, sigma: f32) {
    let (w, h) = (decoded.width, decoded.height);
    for frame in &mut decoded.frames {
        let Some(image) = RgbaImage::from_raw(w, h, frame.rgba.to_vec()) else { continue };
        frame.rgba = Arc::from(image::imageops::fast_blur(&image, sigma).into_raw());
    }
}

/// Decodes `bytes` for `request`, in its tint when it has one.
pub fn decode(bytes: &[u8], request: &DecodeRequest, animation_budget: usize) -> Result<Decoded, String> {
    let mut decoded = decode_untinted(bytes, request, animation_budget)?;
    if request.blur > 0 {
        apply_blur(&mut decoded, request.blur as f32 / 10.);
    }
    if let Some(tint) = request.tint {
        apply_tint(&mut decoded, tint);
    }
    Ok(decoded)
}

/// Animated GIF and WebP frame by frame, SVG rendered at the box size, anything else as a
/// still with its EXIF orientation applied.
fn decode_untinted(bytes: &[u8], request: &DecodeRequest, animation_budget: usize) -> Result<Decoded, String> {
    if is_svg(bytes) {
        return decode_svg(bytes, request);
    }
    let format = image::guess_format(bytes).map_err(|e| e.to_string())?;
    let frames = match format {
        ImageFormat::Gif => {
            let decoder = GifDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
            Some(decoder.into_frames().collect_frames().map_err(|e| e.to_string())?)
        }
        ImageFormat::WebP => {
            let decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
            if decoder.has_animation() {
                Some(decoder.into_frames().collect_frames().map_err(|e| e.to_string())?)
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some(frames) = frames.filter(|f| f.len() > 1) {
        return Ok(animation(frames, request, animation_budget));
    }
    let mut decoder = ImageReader::with_format(Cursor::new(bytes), format).into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().ok();
    let mut image = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    if let Some(o) = orientation {
        image.apply_orientation(o);
    }
    let (sw, sh) = (image.width(), image.height());
    let rgba = resize(image.into_rgba8(), scale_for(sw, sh, request));
    let (width, height) = rgba.dimensions();
    Ok(Decoded {
        width,
        height,
        source_width: sw,
        source_height: sh,
        frames: vec![Frame { rgba: Arc::from(rgba.into_raw()), delay: Duration::ZERO }],
    })
}

fn animation(frames: Vec<image::Frame>, request: &DecodeRequest, budget: usize) -> Decoded {
    let (sw, sh) = frames[0].buffer().dimensions();
    let scale = scale_for(sw, sh, request);
    let delays: Vec<Duration> = frames
        .iter()
        .map(|f| {
            let d: Duration = f.delay().into();
            if d < MIN_FRAME { SLOW_FRAME } else { d }
        })
        .collect();
    let (fw, fh) = (((sw as f32 * scale).round() as usize).max(1), ((sh as f32 * scale).round() as usize).max(1));
    // Thin before resizing: drop every other frame until the decode fits, folding each dropped
    // frame's delay into the kept one so the loop keeps its length.
    let mut stride = 1;
    while frames.len() / stride > 1 && fw * fh * 4 * frames.len().div_ceil(stride) > budget {
        stride *= 2;
    }
    let mut out = Vec::with_capacity(frames.len().div_ceil(stride));
    for (i, frame) in frames.into_iter().enumerate() {
        if i % stride != 0 {
            continue;
        }
        let delay = delays[i..(i + stride).min(delays.len())].iter().sum();
        let rgba = resize(frame.into_buffer(), scale);
        out.push(Frame { rgba: Arc::from(rgba.into_raw()), delay });
    }
    Decoded { width: fw as u32, height: fh as u32, source_width: sw, source_height: sh, frames: out }
}
