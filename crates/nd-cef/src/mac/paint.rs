//! Chromium's frames into surfaces the view owns. CEF's shared IOSurface "cannot be cached":
//! it goes back to Chromium's pool when `on_accelerated_paint` returns, so each frame is copied
//! with one GPU blit into a triple-buffered pool (one shown, one composited by the window
//! server, one being written) and the copy becomes the layer's contents.

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_core_foundation::{CFDictionary, CFRetained};
use objc2_foundation::{NSDictionary, NSNumber, NSString};
use objc2_io_surface::{
    IOSurfaceRef, kIOSurfaceBytesPerElement, kIOSurfaceBytesPerRow, kIOSurfaceHeight, kIOSurfacePixelFormat,
    kIOSurfaceWidth,
};
use objc2_metal::{
    MTLBlitCommandEncoder, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue, MTLCreateSystemDefaultDevice,
    MTLDevice, MTLPixelFormat, MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};

const POOL: usize = 3;
const BGRA: u32 = u32::from_be_bytes(*b"BGRA");

struct Gpu {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
}

thread_local! {
    static GPU: Option<Gpu> = {
        let device = MTLCreateSystemDefaultDevice();
        device.and_then(|device| Some(Gpu { queue: device.newCommandQueue()?, device }))
    };
}

/// One paint target's surfaces (the page, or a `<select>` popup).
#[derive(Default)]
pub(crate) struct Pool {
    surfaces: Vec<CFRetained<IOSurfaceRef>>,
    size: (usize, usize),
    next: usize,
}

fn new_surface(width: usize, height: usize) -> Option<CFRetained<IOSurfaceRef>> {
    let keys: [&NSString; 5] = unsafe {
        [
            cf_key(kIOSurfaceWidth),
            cf_key(kIOSurfaceHeight),
            cf_key(kIOSurfaceBytesPerElement),
            cf_key(kIOSurfaceBytesPerRow),
            cf_key(kIOSurfacePixelFormat),
        ]
    };
    let values = [
        NSNumber::new_usize(width),
        NSNumber::new_usize(height),
        NSNumber::new_usize(4),
        NSNumber::new_usize((width * 4).next_multiple_of(64)),
        NSNumber::new_u32(BGRA),
    ];
    let values: Vec<&AnyObject> = values.iter().map(|v| v.as_ref()).collect();
    let dict = NSDictionary::<NSString, AnyObject>::from_slices(&keys, &values);
    // SAFETY: NSDictionary is toll-free bridged to CFDictionary, with string keys and number
    // values as IOSurfaceCreate expects.
    let cf: &CFDictionary = unsafe { &*(Retained::as_ptr(&dict).cast::<CFDictionary>()) };
    unsafe { IOSurfaceRef::new(cf) }
}

/// SAFETY: CFString is toll-free bridged to NSString.
unsafe fn cf_key(key: &'static objc2_core_foundation::CFString) -> &'static NSString {
    unsafe { &*(std::ptr::from_ref(key).cast::<NSString>()) }
}

fn texture(gpu: &Gpu, surface: &IOSurfaceRef, rgba: bool) -> Option<Retained<ProtocolObject<dyn MTLTexture>>> {
    let format = if rgba { MTLPixelFormat::RGBA8Unorm } else { MTLPixelFormat::BGRA8Unorm };
    let desc = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            format,
            surface.width(),
            surface.height(),
            false,
        )
    };
    desc.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::ShaderWrite);
    gpu.device.newTextureWithDescriptor_iosurface_plane(&desc, surface, 0)
}

impl Pool {
    /// Copies `shared` (CEF's IOSurface) into the next surface of the pool and returns it, as the
    /// object a layer's `contents` takes. Waits for the GPU, so the copy is done before Chromium
    /// reuses its surface.
    pub(crate) fn copy(&mut self, shared: *mut c_void, rgba: bool) -> Option<Retained<AnyObject>> {
        let shared = unsafe { shared.cast::<IOSurfaceRef>().as_ref() }?;
        let size = (shared.width(), shared.height());
        if size.0 == 0 || size.1 == 0 {
            return None;
        }
        if size != self.size {
            self.surfaces = (0..POOL).filter_map(|_| new_surface(size.0, size.1)).collect();
            self.size = size;
            self.next = 0;
        }
        let target = self.surfaces.get(self.next)?.clone();
        self.next = (self.next + 1) % self.surfaces.len().max(1);
        GPU.with(|gpu| {
            let gpu = gpu.as_ref()?;
            let from = texture(gpu, shared, rgba)?;
            let to = texture(gpu, &target, false)?;
            let buffer = gpu.queue.commandBuffer()?;
            let blit = buffer.blitCommandEncoder()?;
            unsafe { blit.copyFromTexture_toTexture(&from, &to) };
            blit.endEncoding();
            buffer.commit();
            buffer.waitUntilCompleted();
            Some(())
        })?;
        // SAFETY: IOSurfaceRef is an object a CALayer takes as contents.
        unsafe { Retained::retain(CFRetained::as_ptr(&target).as_ptr().cast::<AnyObject>()) }
    }

    pub(crate) fn clear(&mut self) {
        self.surfaces.clear();
        self.size = (0, 0);
    }
}
