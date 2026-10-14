//! The Metal side of the AppKit presenter: CoreText faces and cell metrics, a glyph atlas, and
//! one instanced draw per frame of every background, glyph and decoration in the grid.
//!
//! Glyphs are rasterized once per grapheme, style and box size with CoreText (which brings the
//! system's font fallback, colour emoji and combining marks) into an RGBA atlas: white coverage
//! for ordinary text, tinted per instance, or the glyph's own colours for emoji. Sprites and
//! Nerd Font icons come from `sprite` and `icons`, the same masks the GPUI presenter draws.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Rc;

use nd_ui::prelude::Hsla;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::{
    CFAttributedString, CFBoolean, CFDictionary, CFRetained, CFString, CFType, CGFloat, CGPoint, Type,
};
use objc2_core_graphics::{CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo};
use objc2_core_text::{
    CTFont, CTFontSymbolicTraits, CTLine, CTLineBoundsOptions, kCTFontAttributeName,
    kCTForegroundColorFromContextAttributeName,
};
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBlendFactor, MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue, MTLCreateSystemDefaultDevice,
    MTLDevice, MTLLibrary, MTLLoadAction, MTLOrigin, MTLPixelFormat, MTLPrimitiveType, MTLRegion,
    MTLRenderCommandEncoder, MTLRenderPassDescriptor, MTLRenderPipelineDescriptor, MTLRenderPipelineState,
    MTLResourceOptions, MTLSize, MTLStorageMode, MTLStoreAction, MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};

use crate::frame::{Attrs, Cell, CursorShape, Frame, Row, Underline};
use crate::glyph::{self, Ink};
use crate::icons;
use crate::paint::cell_colors;
use crate::sprite::{self, Mask};
use crate::theme::{Rgb, TerminalTheme};

const SHADERS: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct Instance {
    float2 pos;
    float2 size;
    float4 uv;
    float4 color;
    uint kind;
    uint pad0;
    uint pad1;
    uint pad2;
};

struct Out {
    float4 position [[position]];
    float2 uv;
    float4 color;
    uint kind [[flat]];
};

vertex Out vs(uint vid [[vertex_id]], uint iid [[instance_id]],
              const device Instance* instances [[buffer(0)]],
              constant float2& viewport [[buffer(1)]]) {
    Instance i = instances[iid];
    float2 corner = float2(vid & 1, vid >> 1);
    float2 p = i.pos + corner * i.size;
    Out o;
    o.position = float4(p.x / viewport.x * 2.0 - 1.0, 1.0 - p.y / viewport.y * 2.0, 0.0, 1.0);
    o.uv = mix(i.uv.xy, i.uv.zw, corner);
    o.color = i.color;
    o.kind = i.kind;
    return o;
}

fragment float4 fs(Out in [[stage_in]], texture2d<float> atlas [[texture(0)]]) {
    constexpr sampler s(coord::normalized, filter::nearest, address::clamp_to_edge);
    if (in.kind == 0) {
        return float4(in.color.rgb * in.color.a, in.color.a);
    }
    float4 t = atlas.sample(s, in.uv);
    if (in.kind == 1) {
        float a = t.a * in.color.a;
        return float4(in.color.rgb * a, a);
    }
    return t * in.color.a;
}
"#;

/// One quad: a solid rect (`kind` 0), a tinted coverage glyph (1) or a colour glyph (2).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Instance {
    pos: [f32; 2],
    size: [f32; 2],
    uv: [f32; 4],
    color: [f32; 4],
    kind: u32,
    pad: [u32; 3],
}

fn rgba(c: Hsla) -> [f32; 4] {
    let c = c.to_rgb();
    [c.r, c.g, c.b, c.a]
}

/// The grid's faces at the device scale, and the cell they define.
pub(crate) struct Fonts {
    /// Identifies the face, size and scale in the shared atlas's keys.
    pub id: u64,
    regular: CFRetained<CTFont>,
    bold: CFRetained<CTFont>,
    italic: CFRetained<CTFont>,
    bold_italic: CFRetained<CTFont>,
    /// Device pixels.
    pub size: f64,
    pub cell_width: u32,
    pub cell_height: u32,
    pub baseline: f32,
    pub stroke: u32,
}

impl Fonts {
    /// `family` at `points` for a `scale` backing; the system monospaced face when `family` is
    /// `None` or not installed.
    pub fn new(family: Option<&str>, points: f64, scale: f64) -> Self {
        let size = points * scale;
        let regular = family
            // CoreText will not open the system's private families (`.AppleSystemUIFont..`) by
            // name; the monospaced one is what `system_mono` builds.
            .filter(|f| !f.is_empty() && !f.starts_with('.'))
            .and_then(|f| {
                let name = CFString::from_str(f);
                let font = unsafe { CTFont::with_name(&name, size, std::ptr::null()) };
                // CoreText substitutes a default face for unknown names; keep it only when the
                // family matches.
                let got = unsafe { font.family_name() }.to_string();
                (got.eq_ignore_ascii_case(f)).then_some(font)
            })
            .unwrap_or_else(|| system_mono(size));
        let variant = |traits: CTFontSymbolicTraits| {
            unsafe { regular.copy_with_symbolic_traits(size, std::ptr::null(), traits, traits) }
                .unwrap_or_else(|| regular.clone())
        };
        let bold = variant(CTFontSymbolicTraits::TraitBold);
        let italic = variant(CTFontSymbolicTraits::TraitItalic);
        let bold_italic = variant(CTFontSymbolicTraits::TraitBold | CTFontSymbolicTraits::TraitItalic);
        // The widest printable ASCII advance, not the face's maximum: a Nerd Font's icons would
        // make that several cells wide.
        let mut chars: Vec<u16> = (0x20u16..0x7f).collect();
        let mut glyphs = vec![0u16; chars.len()];
        let mut advances = vec![objc2_core_foundation::CGSize::default(); chars.len()];
        unsafe {
            regular.glyphs_for_characters(
                NonNull::new_unchecked(chars.as_mut_ptr()),
                NonNull::new_unchecked(glyphs.as_mut_ptr()),
                chars.len() as isize,
            );
            regular.advances_for_glyphs(
                objc2_core_text::CTFontOrientation::Horizontal,
                NonNull::new_unchecked(glyphs.as_mut_ptr()),
                advances.as_mut_ptr(),
                glyphs.len() as isize,
            );
        }
        let width = advances.iter().map(|a| a.width).fold(0., f64::max).max(1.);
        let (ascent, descent, leading) = unsafe { (regular.ascent(), regular.descent(), regular.leading()) };
        let cell_height = (ascent + descent + leading).ceil().max(1.) as u32;
        let baseline = (ascent + leading / 2.).ceil() as f32;
        let id = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            unsafe { regular.post_script_name() }.to_string().hash(&mut h);
            size.to_bits().hash(&mut h);
            h.finish()
        };
        Self {
            id,
            regular,
            bold,
            italic,
            bold_italic,
            size,
            cell_width: width.ceil() as u32,
            cell_height,
            baseline,
            stroke: (scale.round() as u32).max(1),
        }
    }

    fn pick(&self, attrs: Attrs) -> &CTFont {
        match (attrs.contains(Attrs::BOLD), attrs.contains(Attrs::ITALIC)) {
            (false, false) => &self.regular,
            (true, false) => &self.bold,
            (false, true) => &self.italic,
            (true, true) => &self.bold_italic,
        }
    }

    /// True when the grid's own face has a glyph for `c`.
    fn covers(&self, c: char) -> bool {
        let mut units = [0u16; 2];
        let encoded = c.encode_utf16(&mut units);
        let mut glyphs = [0u16; 2];
        unsafe {
            self.regular.glyphs_for_characters(
                NonNull::new_unchecked(encoded.as_mut_ptr()),
                NonNull::new_unchecked(glyphs.as_mut_ptr()),
                encoded.len() as isize,
            )
        }
    }
}

fn system_mono(size: f64) -> CFRetained<CTFont> {
    // NSFont is toll-free bridged to CTFont: SF Mono, which no family name reaches.
    let ns =
        objc2_app_kit::NSFont::monospacedSystemFontOfSize_weight(size, unsafe { objc2_app_kit::NSFontWeightRegular });
    let ptr = Retained::into_raw(ns).cast::<CTFont>();
    unsafe { CFRetained::from_raw(NonNull::new(ptr).expect("a system font")) }
}

/// Where a glyph sits in the atlas and how its image lies over its cell, in device pixels.
#[derive(Clone, Copy, Debug)]
struct Entry {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    /// The image's top left from the cell's.
    left: i32,
    top: i32,
    color: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    Text { face: u64, text: String, bold: bool, italic: bool, cells: u16 },
    Sprite { face: u64, c: char },
    Icon { face: u64, c: char, cells: u16 },
    Curly { face: u64, cells: u16 },
}

/// A shelf packer over one RGBA texture.
struct Atlas {
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    size: u32,
    entries: HashMap<Key, Option<Entry>>,
    shelf_y: u32,
    shelf_h: u32,
    cursor_x: u32,
}

impl Atlas {
    fn new(device: &ProtocolObject<dyn MTLDevice>, size: u32) -> Self {
        let desc = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::RGBA8Unorm,
                size as usize,
                size as usize,
                false,
            )
        };
        desc.setUsage(MTLTextureUsage::ShaderRead);
        desc.setStorageMode(MTLStorageMode::Shared);
        let texture = device.newTextureWithDescriptor(&desc).expect("glyph atlas texture");
        Self { texture, size, entries: HashMap::new(), shelf_y: 0, shelf_h: 0, cursor_x: 0 }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.shelf_y = 0;
        self.shelf_h = 0;
        self.cursor_x = 0;
    }

    /// Room for a `w` by `h` image, or `None` when the atlas is full.
    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w > self.size || h > self.size {
            return None;
        }
        if self.cursor_x + w > self.size {
            self.shelf_y += self.shelf_h + 1;
            self.shelf_h = 0;
            self.cursor_x = 0;
        }
        if self.shelf_y + h > self.size {
            return None;
        }
        let at = (self.cursor_x, self.shelf_y);
        self.cursor_x += w + 1;
        self.shelf_h = self.shelf_h.max(h);
        Some(at)
    }

    fn upload(&mut self, image: &Image) -> Option<Entry> {
        let (x, y) = self.alloc(image.width, image.height)?;
        let region = MTLRegion {
            origin: MTLOrigin { x: x as usize, y: y as usize, z: 0 },
            size: MTLSize { width: image.width as usize, height: image.height as usize, depth: 1 },
        };
        unsafe {
            self.texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                region,
                0,
                NonNull::new(image.rgba.as_ptr() as *mut c_void).expect("pixels"),
                image.width as usize * 4,
            );
        }
        Some(Entry {
            x,
            y,
            width: image.width,
            height: image.height,
            left: image.left,
            top: image.top,
            color: image.color,
        })
    }
}

/// A rasterized glyph: RGBA pixels and where they lie over the cell.
struct Image {
    width: u32,
    height: u32,
    left: i32,
    top: i32,
    rgba: Vec<u8>,
    color: bool,
}

impl Image {
    fn from_mask(mask: &Mask) -> Self {
        let mut rgba = Vec::with_capacity(mask.alpha.len() * 4);
        for a in &mask.alpha {
            rgba.extend_from_slice(&[*a, *a, *a, *a]);
        }
        Self { width: mask.width, height: mask.height, left: 0, top: 0, rgba, color: false }
    }
}

/// Renders `text` in `font` with CoreText into an image that holds its whole ink, the pen
/// placed by [`glyph::fit`] in a `box_w` by `box_h` box.
fn rasterize_text(text: &str, font: &CTFont, box_w: u32, box_h: u32, baseline: f32, symbol: bool) -> Option<Image> {
    let string = CFString::from_str(text);
    let keys: [&CFString; 2] = unsafe { [kCTFontAttributeName, kCTForegroundColorFromContextAttributeName] };
    let font_value: &CFType = font;
    let from_context: &CFType = CFBoolean::new(true);
    let attributes = CFDictionary::<CFString, CFType>::from_slices(&keys, &[font_value, from_context]);
    let attributed = unsafe { CFAttributedString::new(None, Some(&string), Some(attributes.as_opaque())) }?;
    let line = unsafe { CTLine::with_attributed_string(&attributed) };
    let bounds = unsafe { line.bounds_with_options(CTLineBoundsOptions::UseGlyphPathBounds) };
    let (bounds, color) = if bounds.size.width <= 0. || bounds.size.height <= 0. {
        // Bitmap glyphs (colour emoji) have no paths: their typographic box.
        let (mut ascent, mut descent, mut leading) = (0., 0., 0.);
        let width = unsafe { line.typographic_bounds(&mut ascent, &mut descent, &mut leading) };
        if width <= 0. {
            return None;
        }
        (
            objc2_core_foundation::CGRect::new(
                CGPoint::new(0., -descent),
                objc2_core_foundation::CGSize::new(width, ascent + descent),
            ),
            true,
        )
    } else {
        (bounds, line_has_color(&line))
    };
    // Font bounds are y up from the pen; the fit works y down.
    let ink = Ink {
        x: bounds.origin.x as f32,
        y: -(bounds.origin.y + bounds.size.height) as f32,
        width: bounds.size.width as f32,
        height: bounds.size.height as f32,
    };
    let fit = glyph::fit(ink, box_w as f32, box_h as f32, baseline, symbol || color);
    // The image covers the scaled ink plus a pixel of antialiasing on each side.
    let left = (fit.x + ink.x * fit.scale).floor() as i32 - 1;
    let top = (fit.y + ink.y * fit.scale).floor() as i32 - 1;
    let width = ((ink.width * fit.scale).ceil() as i32 + 3).max(1) as u32;
    let height = ((ink.height * fit.scale).ceil() as i32 + 3).max(1) as u32;
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    let space = CGColorSpace::new_device_rgb()?;
    let ctx = unsafe {
        CGBitmapContextCreate(
            rgba.as_mut_ptr().cast(),
            width as usize,
            height as usize,
            8,
            width as usize * 4,
            Some(&space),
            CGImageAlphaInfo::PremultipliedLast.0,
        )
    }?;
    CGContext::set_should_antialias(Some(&ctx), true);
    CGContext::set_allows_font_smoothing(Some(&ctx), true);
    CGContext::set_should_smooth_fonts(Some(&ctx), true);
    CGContext::set_rgb_fill_color(Some(&ctx), 1., 1., 1., 1.);
    // CoreGraphics is y up with the origin at the bottom left of the image: put the pen where
    // the fit wants it, then scale about it.
    let pen_x = f64::from(fit.x) - f64::from(left);
    let pen_y = f64::from(height as i32) - (f64::from(fit.y) - f64::from(top));
    CGContext::translate_ctm(Some(&ctx), pen_x as CGFloat, pen_y as CGFloat);
    CGContext::scale_ctm(Some(&ctx), f64::from(fit.scale), f64::from(fit.scale));
    CGContext::set_text_position(Some(&ctx), 0., 0.);
    unsafe { line.draw(&ctx) };
    drop(ctx);
    // RGBA from CoreGraphics is premultiplied; coverage glyphs keep it as white coverage.
    Some(Image { width, height, left, top, rgba, color })
}

fn line_has_color(line: &CTLine) -> bool {
    let runs = unsafe { line.glyph_runs() };
    let count = runs.count();
    (0..count).any(|i| {
        let run = unsafe { runs.value_at_index(i) } as *const objc2_core_text::CTRun;
        let Some(run) = (unsafe { run.as_ref() }) else { return false };
        let attributes = unsafe { run.attributes() };
        let key: &CFString = unsafe { kCTFontAttributeName };
        let font = unsafe { attributes.value(key as *const CFString as *const c_void) } as *const CTFont;
        unsafe { font.as_ref() }
            .is_some_and(|f| unsafe { f.symbolic_traits() }.contains(CTFontSymbolicTraits::TraitColorGlyphs))
    })
}

/// A wavy underline one cell wide, as a coverage mask.
fn curly_mask(width: u32, height: u32, stroke: u32) -> Mask {
    let mut alpha = vec![0u8; (width * height) as usize];
    let amplitude = (height as f32 - stroke as f32) / 2.;
    let half = stroke as f32 / 2. + 0.3;
    for y in 0..height {
        for x in 0..width {
            let t = (x as f32 + 0.5) / width as f32 * std::f32::consts::TAU;
            let centre = height as f32 / 2. - t.sin() * amplitude;
            let d = (y as f32 + 0.5 - centre).abs();
            let a = (1. - (d - half).max(0.)).clamp(0., 1.);
            alpha[(y * width + x) as usize] = (a * 255.) as u8;
        }
    }
    Mask { width, height, alpha }
}

/// Everything one frame draws besides the grid itself.
pub(crate) struct Overlay<'a> {
    pub marked: Option<&'a str>,
    pub hover: Option<&'a crate::links::Link>,
}

pub(crate) struct Renderer {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    pipeline: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    atlas: Atlas,
    instances: Vec<Instance>,
}

thread_local! {
    static SHARED: RefCell<Option<Rc<RefCell<Renderer>>>> = const { RefCell::new(None) };
}

impl Renderer {
    /// The main thread's renderer, which every terminal view draws with: one device, pipeline
    /// and atlas however many terminals are open.
    pub fn shared() -> Option<Rc<RefCell<Renderer>>> {
        SHARED.with(|s| {
            let mut s = s.borrow_mut();
            if s.is_none() {
                *s = Self::new().map(|r| Rc::new(RefCell::new(r)));
            }
            s.clone()
        })
    }

    pub fn new() -> Option<Self> {
        let device = MTLCreateSystemDefaultDevice()?;
        let queue = device.newCommandQueue()?;
        let library = device.newLibraryWithSource_options_error(&NSString::from_str(SHADERS), None).ok()?;
        let vs = library.newFunctionWithName(&NSString::from_str("vs"))?;
        let fs = library.newFunctionWithName(&NSString::from_str("fs"))?;
        let desc = MTLRenderPipelineDescriptor::new();
        desc.setVertexFunction(Some(&vs));
        desc.setFragmentFunction(Some(&fs));
        let attachment = unsafe { desc.colorAttachments().objectAtIndexedSubscript(0) };
        attachment.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        attachment.setBlendingEnabled(true);
        attachment.setSourceRGBBlendFactor(MTLBlendFactor::One);
        attachment.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        attachment.setSourceAlphaBlendFactor(MTLBlendFactor::One);
        attachment.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        let pipeline = device.newRenderPipelineStateWithDescriptor_error(&desc).ok()?;
        let atlas = Atlas::new(&device, 2048);
        Some(Self { device, queue, pipeline, atlas, instances: Vec::new() })
    }

    pub fn device(&self) -> &ProtocolObject<dyn MTLDevice> {
        &self.device
    }

    fn glyph(&mut self, key: Key, draw: impl FnOnce() -> Option<Image>) -> Option<Entry> {
        if let Some(entry) = self.atlas.entries.get(&key) {
            return *entry;
        }
        let image = draw();
        let entry = match image {
            Some(image) => match self.atlas.upload(&image) {
                Some(entry) => Some(entry),
                None => {
                    // Full: start over; this frame's glyphs refill it.
                    self.atlas.clear();
                    self.atlas.upload(&image)
                }
            },
            None => None,
        };
        self.atlas.entries.insert(key, entry);
        entry
    }

    fn push_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Hsla) {
        if color.a <= 0. || w <= 0. || h <= 0. {
            return;
        }
        self.instances.push(Instance { pos: [x, y], size: [w, h], color: rgba(color), ..Default::default() });
    }

    fn push_entry(&mut self, e: Entry, x: f32, y: f32, color: Hsla) {
        let s = self.atlas.size as f32;
        self.instances.push(Instance {
            pos: [x + e.left as f32, y + e.top as f32],
            size: [e.width as f32, e.height as f32],
            uv: [e.x as f32 / s, e.y as f32 / s, (e.x + e.width) as f32 / s, (e.y + e.height) as f32 / s],
            color: if e.color { [1., 1., 1., color.a] } else { rgba(color) },
            kind: if e.color { 2 } else { 1 },
            pad: [0; 3],
        });
    }

    /// The instances for `frame` with the grid's top left at `origin` (device pixels).
    pub fn build(
        &mut self,
        frame: &Frame,
        theme: &TerminalTheme,
        fonts: &Fonts,
        origin: (f32, f32),
        overlay: &Overlay<'_>,
    ) {
        self.instances.clear();
        let (cw, ch) = (fonts.cell_width as f32, fonts.cell_height as f32);
        let at = |col: u16, row: u16| (origin.0 + f32::from(col) * cw, origin.1 + f32::from(row) * ch);
        let cursor = frame.cursor;
        let block = cursor.filter(|c| c.shape == CursorShape::Block && overlay.marked.is_none());
        for (y, row) in frame.rows.iter().enumerate() {
            let mut run: Option<(u16, u16, Hsla)> = None;
            for (x, cell) in row.cells.iter().enumerate() {
                let bg = cell_colors(cell, theme, frame).1;
                match (&mut run, bg) {
                    (Some((_, end, c)), Some(bg)) if *c == bg && *end == x as u16 => *end += 1,
                    (_, bg) => {
                        if let Some((start, end, c)) = run.take() {
                            let (px, py) = at(start, y as u16);
                            self.push_rect(px, py, f32::from(end - start) * cw, ch, c);
                        }
                        run = bg.map(|c| (x as u16, x as u16 + 1, c));
                    }
                }
            }
            if let Some((start, end, c)) = run {
                let (px, py) = at(start, y as u16);
                self.push_rect(px, py, f32::from(end - start) * cw, ch, c);
            }
        }
        let cursor_color = cursor.and_then(|c| c.color).map_or(theme.cursor, Rgb::to_hsla);
        if let Some(c) = block {
            let width = frame.cell(c.col, c.row).map_or(1, |(_, cell)| cell.width());
            let (px, py) = at(c.col, c.row);
            self.push_rect(px, py, f32::from(width) * cw, ch, cursor_color);
        }
        for (y, row) in frame.rows.iter().enumerate() {
            for (x, cell) in row.cells.iter().enumerate() {
                if cell.is_blank() || cell.attrs.contains(Attrs::INVISIBLE) {
                    continue;
                }
                let (mut fg, _) = cell_colors(cell, theme, frame);
                if block.is_some_and(|c| usize::from(c.col) == x && usize::from(c.row) == y) {
                    fg = theme.background;
                }
                let (px, py) = at(x as u16, y as u16);
                self.push_cell(row, x, cell, fg, fonts, px, py);
                self.push_decorations(cell, fg, theme, fonts, px, py);
            }
        }
        if let Some(c) = cursor.filter(|c| c.shape != CursorShape::Block || overlay.marked.is_some()) {
            let (px, py) = at(c.col, c.row);
            let s = fonts.stroke as f32;
            match c.shape {
                CursorShape::Bar => self.push_rect(px, py, s * 2., ch, cursor_color),
                CursorShape::Underline => self.push_rect(px, py + ch - s * 2., cw, s * 2., cursor_color),
                _ => {
                    self.push_rect(px, py, cw, s, cursor_color);
                    self.push_rect(px, py + ch - s, cw, s, cursor_color);
                    self.push_rect(px, py, s, ch, cursor_color);
                    self.push_rect(px + cw - s, py, s, ch, cursor_color);
                }
            }
        }
        if let Some(link) = overlay.hover {
            let (px, py) = at(link.start, link.row);
            let s = fonts.stroke as f32;
            self.push_rect(
                px,
                py + fonts.baseline + s * 2.,
                f32::from(link.end - link.start) * cw,
                s,
                theme.foreground,
            );
        }
        if let (Some(marked), Some(c)) = (overlay.marked, cursor) {
            let (px, py) = at(c.col, c.row);
            let cells = marked.chars().count().max(1) as u16 * 2;
            let key = Key::Text { face: fonts.id, text: marked.to_owned(), bold: false, italic: false, cells };
            let font = fonts.regular.clone();
            let (bw, bh, base) = (u32::from(cells) * fonts.cell_width, fonts.cell_height, fonts.baseline);
            if let Some(e) = self.glyph(key, || rasterize_text(marked, &font, bw, bh, base, false)) {
                let width = (e.left + e.width as i32).max(fonts.cell_width as i32) as f32;
                self.push_rect(px, py, width, ch, theme.background);
                self.push_entry(e, px, py, theme.foreground);
                self.push_rect(
                    px,
                    py + fonts.baseline + fonts.stroke as f32 * 2.,
                    width,
                    fonts.stroke as f32,
                    theme.foreground,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_cell(&mut self, row: &Row, x: usize, cell: &Cell, fg: Hsla, fonts: &Fonts, px: f32, py: f32) {
        let g = row.grapheme(cell);
        let mut chars = g.chars();
        let single = match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        };
        let (cw, ch) = (fonts.cell_width, fonts.cell_height);
        if let Some(c) = single.filter(|c| sprite::is_sprite(*c)) {
            let stroke = fonts.stroke;
            if let Some(e) = self.glyph(Key::Sprite { face: fonts.id, c }, || {
                sprite::draw(c, cw, ch, stroke).map(|m| Image::from_mask(&m))
            }) {
                self.push_entry(e, px, py, fg);
            }
            return;
        }
        let cells = glyph::box_cells(row, x);
        if let Some(c) = single.filter(|c| glyph::is_symbol(*c) && icons::covers(*c) && !fonts.covers(*c)) {
            let (ppem, base) = (fonts.size as f32, fonts.baseline);
            let key = Key::Icon { face: fonts.id, c, cells };
            if let Some(e) =
                self.glyph(key, || icons::mask(c, ppem, u32::from(cells) * cw, ch, base).map(|m| Image::from_mask(&m)))
            {
                self.push_entry(e, px, py, fg);
            }
            return;
        }
        let bold = cell.attrs.contains(Attrs::BOLD);
        let italic = cell.attrs.contains(Attrs::ITALIC);
        let key = Key::Text { face: fonts.id, text: g.to_owned(), bold, italic, cells };
        let font = fonts.pick(cell.attrs).retain();
        let symbol = single.is_some_and(glyph::is_symbol);
        let (bw, base) = (u32::from(cells) * cw, fonts.baseline);
        if let Some(e) = self.glyph(key, || rasterize_text(g, &font, bw, ch, base, symbol)) {
            self.push_entry(e, px, py, fg);
        }
    }

    fn push_decorations(&mut self, cell: &Cell, fg: Hsla, theme: &TerminalTheme, fonts: &Fonts, px: f32, py: f32) {
        let w = (fonts.cell_width * u32::from(cell.width())) as f32;
        let s = fonts.stroke as f32;
        let color = cell.underline_color.map_or(fg, |c| theme.ink(c));
        let under = py + fonts.baseline + s * 2.;
        match cell.underline {
            Underline::None => {}
            Underline::Single => self.push_rect(px, under, w, s, color),
            Underline::Double => {
                self.push_rect(px, under, w, s, color);
                self.push_rect(px, under + s * 2., w, s, color);
            }
            Underline::Curly => {
                let (cw, h, stroke) = (fonts.cell_width, (s * 3.) as u32, fonts.stroke);
                if let Some(e) = self.glyph(Key::Curly { face: fonts.id, cells: cell.width() }, || {
                    Some(Image::from_mask(&curly_mask(cw, h, stroke)))
                }) {
                    for i in 0..cell.width() {
                        self.push_entry(e, px + f32::from(i) * cw as f32, under - s, color);
                    }
                }
            }
            Underline::Dotted | Underline::Dashed => {
                let dash = if cell.underline == Underline::Dotted { s * 2. } else { w / 3. };
                let mut x = px;
                while x < px + w {
                    self.push_rect(x, under, dash.min(px + w - x), s, color);
                    x += dash * 2.;
                }
            }
        }
        if cell.attrs.contains(Attrs::STRIKETHROUGH) {
            self.push_rect(px, py + (fonts.cell_height as f32 - s) / 2., w, s, fg);
        }
        if cell.attrs.contains(Attrs::OVERLINE) {
            self.push_rect(px, py, w, s, fg);
        }
        if cell.attrs.contains(Attrs::LINK) && cell.underline == Underline::None {
            self.push_rect(px, under, w, s, nd_theme::color::with_alpha(fg, fg.a * 0.4));
        }
    }

    /// Encodes the built instances into `target`, cleared to `clear` first.
    pub fn encode(
        &self,
        command_buffer: &ProtocolObject<dyn MTLCommandBuffer>,
        target: &ProtocolObject<dyn MTLTexture>,
        clear: Hsla,
    ) {
        let pass = MTLRenderPassDescriptor::renderPassDescriptor();
        let attachment = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
        attachment.setTexture(Some(target));
        attachment.setLoadAction(MTLLoadAction::Clear);
        attachment.setStoreAction(MTLStoreAction::Store);
        let c = clear.to_rgb();
        let a = f64::from(c.a);
        attachment.setClearColor(MTLClearColor {
            red: f64::from(c.r) * a,
            green: f64::from(c.g) * a,
            blue: f64::from(c.b) * a,
            alpha: a,
        });
        let Some(encoder) = command_buffer.renderCommandEncoderWithDescriptor(&pass) else { return };
        if !self.instances.is_empty() {
            let bytes = std::mem::size_of_val(self.instances.as_slice());
            let buffer = unsafe {
                self.device.newBufferWithBytes_length_options(
                    NonNull::new(self.instances.as_ptr() as *mut c_void).expect("instances"),
                    bytes,
                    MTLResourceOptions::StorageModeShared,
                )
            };
            if let Some(buffer) = buffer {
                let viewport = [target.width() as f32, target.height() as f32];
                encoder.setRenderPipelineState(&self.pipeline);
                unsafe {
                    encoder.setVertexBuffer_offset_atIndex(Some(&buffer), 0, 0);
                    encoder.setVertexBytes_length_atIndex(
                        NonNull::new(viewport.as_ptr() as *mut c_void).expect("viewport"),
                        std::mem::size_of_val(&viewport),
                        1,
                    );
                    encoder.setFragmentTexture_atIndex(Some(&self.atlas.texture), 0);
                    encoder.drawPrimitives_vertexStart_vertexCount_instanceCount(
                        MTLPrimitiveType::TriangleStrip,
                        0,
                        4,
                        self.instances.len(),
                    );
                }
            }
        }
        encoder.endEncoding();
    }

    pub fn queue(&self) -> &ProtocolObject<dyn MTLCommandQueue> {
        &self.queue
    }

    /// A `width` by `height` texture to draw into without a view.
    pub fn target(&self, width: u32, height: u32) -> Option<Retained<ProtocolObject<dyn MTLTexture>>> {
        let desc = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm,
                width as usize,
                height as usize,
                false,
            )
        };
        desc.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
        desc.setStorageMode(MTLStorageMode::Shared);
        self.device.newTextureWithDescriptor(&desc)
    }

    /// Draws the built instances into `target` and waits for the GPU to finish.
    pub fn draw_to(&self, target: &ProtocolObject<dyn MTLTexture>, clear: Hsla) {
        let Some(command_buffer) = self.queue.commandBuffer() else { return };
        self.encode(&command_buffer, target, clear);
        command_buffer.commit();
        command_buffer.waitUntilCompleted();
    }

    /// `target`'s pixels as RGBA rows.
    pub fn read(target: &ProtocolObject<dyn MTLTexture>) -> (u32, u32, Vec<u8>) {
        let (width, height) = (target.width() as u32, target.height() as u32);
        let mut bgra = vec![0u8; (width * height * 4) as usize];
        unsafe {
            target.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                NonNull::new(bgra.as_mut_ptr().cast()).expect("pixels"),
                width as usize * 4,
                MTLRegion {
                    origin: MTLOrigin { x: 0, y: 0, z: 0 },
                    size: MTLSize { width: width as usize, height: height as usize, depth: 1 },
                },
                0,
            );
        }
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        (width, height, bgra)
    }
}
