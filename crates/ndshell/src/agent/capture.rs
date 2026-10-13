//! Windows.Graphics.Capture: stills of a window or monitor, and recordings of every frame DWM
//! composes for a window, read back to the CPU and measured as they arrive (a 240 Hz recording
//! kept as pixels would be gigabytes).

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN};
use windows::core::Interface;

pub enum Target {
    Window(HWND),
    Monitor(HMONITOR),
}

/// Tightly packed BGRA rows.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl Image {
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.bgra[i + 2], self.bgra[i + 1], self.bgra[i], self.bgra[i + 3]]
    }

    pub fn crop(&self, x: u32, y: u32, width: u32, height: u32) -> Image {
        let x = x.min(self.width);
        let y = y.min(self.height);
        let width = width.min(self.width - x);
        let height = height.min(self.height - y);
        let mut bgra = Vec::with_capacity((width * height * 4) as usize);
        for row in y..y + height {
            let start = ((row * self.width + x) * 4) as usize;
            bgra.extend_from_slice(&self.bgra[start..start + (width * 4) as usize]);
        }
        Image { width, height, bgra }
    }

    /// Mean RGB of each cell of an `n` by `n` grid, row by row: enough to tell two captures apart.
    pub fn grid(&self, n: u32) -> Vec<[u8; 3]> {
        let mut cells = Vec::with_capacity((n * n) as usize);
        for gy in 0..n {
            for gx in 0..n {
                let (x0, x1) = (self.width * gx / n, self.width * (gx + 1) / n);
                let (y0, y1) = (self.height * gy / n, self.height * (gy + 1) / n);
                let mut sum = [0u64; 3];
                let mut count = 0u64;
                for y in (y0..y1).step_by(2) {
                    for x in (x0..x1).step_by(2) {
                        let [r, g, b, _] = self.pixel(x, y);
                        sum = [sum[0] + r as u64, sum[1] + g as u64, sum[2] + b as u64];
                        count += 1;
                    }
                }
                let count = count.max(1);
                cells.push([(sum[0] / count) as u8, (sum[1] / count) as u8, (sum[2] / count) as u8]);
            }
        }
        cells
    }

    pub fn save_png(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut rgba = self.bgra.clone();
        for px in rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgba).map_err(|e| e.to_string())
    }
}

struct Gpu {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    winrt: IDirect3DDevice,
    staging: Option<(u32, u32, ID3D11Texture2D)>,
}

// SAFETY: a `Gpu` lives behind one mutex, so the immediate context is used by one thread at a
// time; the device is free-threaded.
unsafe impl Send for Gpu {}

impl Gpu {
    fn new() -> Result<Self, String> {
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(|e| format!("D3D11CreateDevice: {e}"))?;
        let device: ID3D11Device = device.ok_or("no D3D11 device")?;
        let dxgi: IDXGIDevice = device.cast().map_err(|e| e.to_string())?;
        let winrt: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
            .and_then(|d| d.cast())
            .map_err(|e| format!("WinRT D3D device: {e}"))?;
        Ok(Self { device, context: context.ok_or("no D3D11 context")?, winrt, staging: None })
    }

    /// Copies the frame's content rectangle into a CPU image.
    fn read(&mut self, frame: &Direct3D11CaptureFrame) -> Result<Image, String> {
        let size = frame.ContentSize().map_err(|e| e.to_string())?;
        let surface = frame.Surface().map_err(|e| e.to_string())?;
        let access: IDirect3DDxgiInterfaceAccess = surface.cast().map_err(|e| e.to_string())?;
        let texture: ID3D11Texture2D = unsafe { access.GetInterface() }.map_err(|e| e.to_string())?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut desc) };
        let width = (size.Width.max(0) as u32).min(desc.Width);
        let height = (size.Height.max(0) as u32).min(desc.Height);
        if width == 0 || height == 0 {
            return Err("empty frame".into());
        }
        let staging = match &self.staging {
            Some((w, h, t)) if *w == desc.Width && *h == desc.Height => t.clone(),
            _ => {
                let staging_desc = D3D11_TEXTURE2D_DESC {
                    Width: desc.Width,
                    Height: desc.Height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };
                let mut t = None;
                unsafe { self.device.CreateTexture2D(&staging_desc, None, Some(&mut t)) }
                    .map_err(|e| format!("staging texture: {e}"))?;
                let t = t.ok_or("no staging texture")?;
                self.staging = Some((desc.Width, desc.Height, t.clone()));
                t
            }
        };
        let region = D3D11_BOX { left: 0, top: 0, front: 0, right: width, bottom: height, back: 1 };
        let mut bgra = Vec::with_capacity((width * height * 4) as usize);
        unsafe {
            self.context.CopySubresourceRegion(&staging, 0, 0, 0, 0, &texture, 0, Some(&region));
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).map_err(|e| format!("Map: {e}"))?;
            let base = mapped.pData as *const u8;
            for row in 0..height as usize {
                let line = std::slice::from_raw_parts(base.add(row * mapped.RowPitch as usize), width as usize * 4);
                bgra.extend_from_slice(line);
            }
            self.context.Unmap(&staging, 0);
        }
        Ok(Image { width, height, bgra })
    }
}

fn item(target: &Target) -> Result<GraphicsCaptureItem, String> {
    let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
        .map_err(|e| format!("capture interop: {e}"))?;
    unsafe {
        match target {
            Target::Window(hwnd) => interop.CreateForWindow(*hwnd),
            Target::Monitor(monitor) => interop.CreateForMonitor(*monitor),
        }
    }
    .map_err(|e| format!("capture item: {e}"))
}

fn session(pool: &Direct3D11CaptureFramePool, item: &GraphicsCaptureItem) -> Result<GraphicsCaptureSession, String> {
    let session = pool.CreateCaptureSession(item).map_err(|e| e.to_string())?;
    let _ = session.SetIsCursorCaptureEnabled(false);
    // Unpackaged apps may be refused borderless capture; the yellow border is then drawn around
    // the window on screen but never into the frames.
    let _ = session.SetIsBorderRequired(false);
    Ok(session)
}

/// One frame of `target`.
pub fn still(target: &Target, timeout: Duration) -> Result<Image, String> {
    let mut gpu = Gpu::new()?;
    let item = item(target)?;
    let size = item.Size().map_err(|e| e.to_string())?;
    let pool =
        Direct3D11CaptureFramePool::CreateFreeThreaded(&gpu.winrt, DirectXPixelFormat::B8G8R8A8UIntNormalized, 1, size)
            .map_err(|e| e.to_string())?;
    let session = session(&pool, &item)?;
    session.StartCapture().map_err(|e| format!("StartCapture: {e}"))?;
    let end = Instant::now() + timeout;
    let image = loop {
        if let Ok(frame) = pool.TryGetNextFrame() {
            break gpu.read(&frame);
        }
        if Instant::now() > end {
            break Err("no frame within the timeout".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let _ = session.Close();
    let _ = pool.Close();
    image
}

/// What a recording measures in each frame: how far in from the right and bottom edges the
/// first `key`-coloured pixel sits. Content that tracks the window edge keeps both constant
/// through a resize; a late frame shifts them or loses the key.
#[derive(Clone, Copy, Debug)]
pub struct EdgeProbe {
    pub key: [u8; 3],
    pub tolerance: u8,
    /// How far in to look before giving up.
    pub scan: u32,
    /// Rows from the top to skip (title row, rounded corner) and columns from the left.
    pub skip: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct FrameStat {
    pub t_ms: f64,
    pub width: u32,
    pub height: u32,
    /// Distance of the key from the right edge, per sampled row: min and max.
    pub right: Option<(u32, u32)>,
    pub bottom: Option<(u32, u32)>,
}

impl EdgeProbe {
    fn matches(&self, [r, g, b, _]: [u8; 4]) -> bool {
        let close = |a: u8, k: u8| a.abs_diff(k) <= self.tolerance;
        close(r, self.key[0]) && close(g, self.key[1]) && close(b, self.key[2])
    }

    fn measure(&self, image: &Image, t_ms: f64) -> FrameStat {
        let (w, h) = (image.width, image.height);
        let spread = |samples: Vec<Option<u32>>| -> Option<(u32, u32)> {
            let found: Vec<u32> = samples.into_iter().collect::<Option<_>>()?;
            Some((*found.iter().min()?, *found.iter().max()?))
        };
        let rows: Vec<u32> = (1..=5).map(|i| self.skip + (h.saturating_sub(2 * self.skip)) * i / 6).collect();
        let cols: Vec<u32> = (1..=5).map(|i| self.skip + (w.saturating_sub(2 * self.skip)) * i / 6).collect();
        let right = rows
            .iter()
            .map(|&y| (0..self.scan.min(w)).find(|&d| y < h && self.matches(image.pixel(w - 1 - d, y))))
            .collect();
        let bottom = cols
            .iter()
            .map(|&x| (0..self.scan.min(h)).find(|&d| x < w && self.matches(image.pixel(x, h - 1 - d))))
            .collect();
        FrameStat { t_ms, width: w, height: h, right: spread(right), bottom: spread(bottom) }
    }
}

struct RecordState {
    gpu: Gpu,
    probe: EdgeProbe,
    started: Instant,
    frames: Vec<FrameStat>,
    /// Keep frame number n as a PNG at this path.
    keep: Vec<(usize, std::path::PathBuf)>,
    errors: u32,
}

/// Every frame DWM composes for a window, measured by an [`EdgeProbe`] as it arrives.
pub struct Recording {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    state: Arc<Mutex<RecordState>>,
}

impl Recording {
    pub fn start(hwnd: HWND, probe: EdgeProbe, keep: Vec<(usize, std::path::PathBuf)>) -> Result<Self, String> {
        let gpu = Gpu::new()?;
        let item = item(&Target::Window(hwnd))?;
        // Buffers as large as the whole desktop, so a window growing mid-recording is never cropped.
        let max = unsafe {
            SizeInt32 { Width: GetSystemMetrics(SM_CXVIRTUALSCREEN), Height: GetSystemMetrics(SM_CYVIRTUALSCREEN) }
        };
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &gpu.winrt,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            3,
            max,
        )
        .map_err(|e| e.to_string())?;
        let state = Arc::new(Mutex::new(RecordState {
            gpu,
            probe,
            started: Instant::now(),
            frames: Vec::new(),
            keep,
            errors: 0,
        }));
        let sink = state.clone();
        pool.FrameArrived(&TypedEventHandler::new(move |pool: windows::core::Ref<Direct3D11CaptureFramePool>, _| {
            let Some(pool) = pool.as_ref() else { return Ok(()) };
            let Ok(frame) = pool.TryGetNextFrame() else { return Ok(()) };
            let mut state = sink.lock().unwrap();
            let t_ms = state.started.elapsed().as_secs_f64() * 1000.;
            match state.gpu.read(&frame) {
                Ok(image) => {
                    let stat = state.probe.measure(&image, t_ms);
                    let n = state.frames.len();
                    if let Some(i) = state.keep.iter().position(|(k, _)| *k == n) {
                        let (_, path) = state.keep.remove(i);
                        let _ = image.save_png(&path);
                    }
                    state.frames.push(stat);
                }
                Err(_) => state.errors += 1,
            }
            Ok(())
        }))
        .map_err(|e| e.to_string())?;
        let session = session(&pool, &item)?;
        session.StartCapture().map_err(|e| format!("StartCapture: {e}"))?;
        Ok(Self { pool, session, state })
    }

    /// Frames so far.
    pub fn count(&self) -> usize {
        self.state.lock().unwrap().frames.len()
    }

    pub fn stop(self) -> (Vec<FrameStat>, u32) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
        let mut state = self.state.lock().unwrap();
        (std::mem::take(&mut state.frames), state.errors)
    }
}
