//! Windows: Media Foundation's media engine in frame-server mode. It decodes on the GPU (H.264,
//! and HEVC with the HEVC Video Extensions), plays the sound and keeps the clock; this thread asks
//! it each few milliseconds whether a new frame is due, has it drawn into a D3D11 texture at the
//! size the box needs, and reads that back for GPUI's sprite atlas.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::Duration;

use windows::Win32::Foundation::{HMODULE, RECT, S_OK};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Media::MediaFoundation::{
    CLSID_MFMediaEngineClassFactory, IMFAttributes, IMFDXGIDeviceManager, IMFMediaEngine, IMFMediaEngineClassFactory,
    IMFMediaEngineNotify, IMFMediaEngineNotify_Impl, MF_MEDIA_ENGINE_CALLBACK, MF_MEDIA_ENGINE_DXGI_MANAGER,
    MF_MEDIA_ENGINE_EVENT_ENDED, MF_MEDIA_ENGINE_EVENT_ERROR, MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA,
    MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, MF_VERSION, MFARGB, MFCreateAttributes, MFCreateDXGIDeviceManager,
    MFSTARTUP_FULL, MFShutdown, MFStartup,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::core::{BSTR, Interface};
use windows_core::implement;

use super::{Command, Event, Frame, Shared, fit_size};

/// How often the thread asks the engine for a frame: under a 120 Hz frame.
const TICK: Duration = Duration::from_millis(4);

pub(super) fn open(source: &str, shared: Arc<Shared>) -> Result<Sender<Command>, String> {
    let (tx, rx) = channel();
    let source = source.to_owned();
    std::thread::Builder::new()
        .name("nd-video".into())
        .spawn(move || {
            // SAFETY: this thread's COM apartment and Media Foundation reference are its own.
            unsafe {
                let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
                let started = MFStartup(MF_VERSION, MFSTARTUP_FULL).is_ok();
                if let Err(error) = run(&source, &rx, &shared) {
                    shared.event(Event::Failed(error));
                }
                if started {
                    let _ = MFShutdown();
                }
                if com {
                    CoUninitialize();
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(tx)
}

#[implement(IMFMediaEngineNotify)]
struct Notify {
    events: Sender<u32>,
}

impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, _param1: usize, _param2: u32) -> windows::core::Result<()> {
        let _ = self.events.send(event);
        Ok(())
    }
}

/// The texture the engine draws into and its CPU-readable copy, at one size.
struct Target {
    size: (u32, u32),
    texture: ID3D11Texture2D,
    staging: ID3D11Texture2D,
}

struct Engine {
    engine: IMFMediaEngine,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    target: Option<Target>,
    native: Option<(u32, u32)>,
    fit: Option<(u32, u32, bool)>,
    looping: bool,
}

impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: the engine is ours; Shutdown releases its sources and threads.
        unsafe {
            let _ = self.engine.Shutdown();
        }
    }
}

fn hr(context: &str) -> impl Fn(windows::core::Error) -> String + '_ {
    move |e| format!("{context}: {}", e.message())
}

unsafe fn create(source: &str, events: Sender<u32>) -> Result<Engine, String> {
    unsafe {
        let mut device = None;
        let mut context = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .map_err(hr("no Direct3D 11 device"))?;
        let (device, context): (ID3D11Device, ID3D11DeviceContext) =
            (device.ok_or("no Direct3D 11 device")?, context.ok_or("no Direct3D 11 context")?);
        // The engine's decoder threads share the device with this one.
        let _ =
            device.cast::<ID3D11Multithread>().map_err(hr("Direct3D 11 multithreading"))?.SetMultithreadProtected(true);
        let mut token = 0;
        let mut manager: Option<IMFDXGIDeviceManager> = None;
        MFCreateDXGIDeviceManager(&mut token, &mut manager).map_err(hr("no DXGI device manager"))?;
        let manager = manager.ok_or("no DXGI device manager")?;
        manager.ResetDevice(&device, token).map_err(hr("DXGI device manager"))?;

        let notify: IMFMediaEngineNotify = Notify { events }.into();
        let mut attributes: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attributes, 3).map_err(hr("media engine attributes"))?;
        let attributes = attributes.ok_or("media engine attributes")?;
        attributes.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify).map_err(hr("media engine callback"))?;
        attributes.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &manager).map_err(hr("media engine device"))?;
        attributes
            .SetUINT32(&MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32)
            .map_err(hr("media engine format"))?;
        let factory: IMFMediaEngineClassFactory =
            CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)
                .map_err(hr("no media engine"))?;
        let engine = factory.CreateInstance(0, &attributes).map_err(hr("no media engine"))?;
        engine.SetSource(&BSTR::from(source)).map_err(hr("the video could not be opened"))?;
        Ok(Engine { engine, device, context, target: None, native: None, fit: None, looping: false })
    }
}

fn run(source: &str, commands: &Receiver<Command>, shared: &Shared) -> Result<(), String> {
    let (tx, events) = channel();
    // SAFETY: COM and Media Foundation are started on this thread (`open`).
    let mut engine = unsafe { create(source, tx)? };
    loop {
        loop {
            match commands.try_recv() {
                Ok(command) => engine.command(command)?,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
        while let Ok(event) = events.try_recv() {
            engine.event(event as i32, shared)?;
        }
        if engine.native.is_some() {
            engine.frame(shared)?;
        }
        std::thread::sleep(TICK);
    }
}

impl Engine {
    fn command(&mut self, command: Command) -> Result<(), String> {
        // SAFETY: calls on a live engine from the thread that made it.
        unsafe {
            let e = &self.engine;
            match command {
                Command::Playing(true) => e.Play().map_err(hr("play"))?,
                Command::Playing(false) => e.Pause().map_err(hr("pause"))?,
                Command::Muted(muted) => e.SetMuted(muted).map_err(hr("mute"))?,
                Command::Looping(looping) => {
                    self.looping = looping;
                    e.SetLoop(looping).map_err(hr("loop"))?
                }
                Command::Seek(seconds) => e.SetCurrentTime(seconds).map_err(hr("seek"))?,
                Command::Fit { width, height, cover } => self.fit = Some((width, height, cover)),
            }
        }
        Ok(())
    }

    fn event(&mut self, event: i32, shared: &Shared) -> Result<(), String> {
        // SAFETY: calls on a live engine from the thread that made it.
        unsafe {
            if event == MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA.0 {
                let (mut w, mut h) = (0, 0);
                if self.engine.HasVideo().as_bool() {
                    self.engine.GetNativeVideoSize(Some(&mut w), Some(&mut h)).map_err(hr("video size"))?;
                }
                let duration = self.engine.GetDuration();
                self.native = Some((w, h));
                shared.event(Event::Ready { width: w, height: h, duration: duration.is_finite().then_some(duration) });
            } else if event == MF_MEDIA_ENGINE_EVENT_ENDED.0 && !self.looping {
                shared.event(Event::Ended);
            } else if event == MF_MEDIA_ENGINE_EVENT_ERROR.0 {
                let code = self.engine.GetError().map(|e| (e.GetErrorCode(), e.GetExtendedErrorCode())).ok();
                return Err(match code {
                    Some((4, _)) => "this video's format is not supported".into(),
                    Some((3, Err(e))) => format!("the video could not be decoded: {}", e.message()),
                    Some((_, Err(e))) => format!("the video could not be played: {}", e.message()),
                    _ => "the video could not be played".into(),
                });
            }
        }
        Ok(())
    }

    /// Draws and reads back the frame the engine says is due, if there is one.
    fn frame(&mut self, shared: &Shared) -> Result<(), String> {
        let Some(native) = self.native.filter(|(w, h)| *w > 0 && *h > 0) else { return Ok(()) };
        let mut pts = 0i64;
        // SAFETY: a live engine. The binding folds S_FALSE ("no new frame") into Ok, so the
        // HRESULT is read from the vtable.
        let due = unsafe {
            (Interface::vtable(&self.engine).OnVideoStreamTick)(Interface::as_raw(&self.engine), &mut pts) == S_OK
        };
        if !due {
            return Ok(());
        }
        let size = fit_size(native, self.fit);
        if self.target.as_ref().map(|t| t.size) != Some(size) {
            self.target = Some(self.allocate(size)?);
        }
        let target = self.target.as_ref().expect("allocated above");
        let rect = RECT { left: 0, top: 0, right: size.0 as i32, bottom: size.1 as i32 };
        let border = MFARGB { rgbBlue: 0, rgbGreen: 0, rgbRed: 0, rgbAlpha: 255 };
        // SAFETY: both textures are live and of `size`; the mapping is read inside its pitch.
        unsafe {
            self.engine.TransferVideoFrame(&target.texture, None, &rect, Some(&border)).map_err(hr("video frame"))?;
            self.context.CopyResource(&target.staging, &target.texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(&target.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).map_err(hr("video frame"))?;
            let row = size.0 as usize * 4;
            let mut bgra = Vec::with_capacity(row * size.1 as usize);
            for y in 0..size.1 as usize {
                let line = (mapped.pData as *const u8).add(y * mapped.RowPitch as usize);
                bgra.extend_from_slice(std::slice::from_raw_parts(line, row));
            }
            self.context.Unmap(&target.staging, 0);
            shared.frame(Frame { width: size.0, height: size.1, bgra, position: pts as f64 / 10_000_000. });
        }
        Ok(())
    }

    fn allocate(&self, (width, height): (u32, u32)) -> Result<Target, String> {
        let mut desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let (mut texture, mut staging) = (None, None);
        // SAFETY: plain 2D texture descriptions on a live device.
        unsafe {
            self.device.CreateTexture2D(&desc, None, Some(&mut texture)).map_err(hr("video texture"))?;
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            self.device.CreateTexture2D(&desc, None, Some(&mut staging)).map_err(hr("video texture"))?;
        }
        Ok(Target {
            size: (width, height),
            texture: texture.ok_or("video texture")?,
            staging: staging.ok_or("video texture")?,
        })
    }
}
