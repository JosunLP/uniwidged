//! One device, many surfaces.
//!
//! This crate is the reason uniwidged is a single process rather than one
//! process per widget. A wgpu device and its driver context cost roughly
//! 30–60 MB and a non-trivial start-up; paying that once for eight widgets is
//! the difference between a widget framework and eight applications pretending
//! to be one.
//!
//! ## What a frame costs
//!
//! Each dirty surface is rasterised by Vello's compute pipeline into its own
//! intermediate storage texture. The indirection is not optional: Vello writes
//! from a compute shader into a *storage* texture, and swapchain images are
//! generally not valid storage targets.
//!
//! Then every dirty surface's blit is recorded onto **one** `CommandEncoder`
//! and submitted once. Clean surfaces are not rasterised, not encoded, not
//! submitted and not presented — they keep the frame they already have, at no
//! cost at all. That is the whole point: eight widgets of which one ticked
//! costs one widget's worth of work.
//!
//! The original plan said one `queue.submit` for the entire frame. That is not
//! reachable through Vello 0.9's public API: `Renderer::render_to_texture`
//! submits its own compute pass internally and offers no way to hand it an
//! encoder. So a frame costs **one submit per dirty surface, plus one for all
//! the blits together** — still constant in the number of *clean* widgets,
//! which is the property that actually matters. [`FrameStats`] reports the
//! real number rather than the hoped-for one.
//!
//! ## Two ways to present
//!
//! [`Presentation::Swapchain`] hands pixels to the compositor directly, which
//! is what Wayland, Windows and macOS all want.
//!
//! [`Presentation::Readback`] renders to a texture, reads the pixels back and
//! lets the caller composite them. Two very different things need it: X11,
//! where a Vulkan swapchain frequently cannot do per-pixel alpha
//! (`ARCHITECTURE.md` §10.1), and every headless test in CI. Rasterisation is
//! identical in both — the same scene, the same GPU, the same pixels — so a
//! golden image captured headlessly is evidence about what ships.

#![allow(clippy::too_many_arguments)]

mod blit;
mod readback;
mod surface;

pub use surface::{Presentation, SurfaceId};

/// Likewise for the scene type every caller has to build, and for `vello`
/// itself — `Glyph` and `Renderer` live there, and a caller reaching them
/// through a second `vello` dependency would get a second `Scene` type.
pub use vello;
/// Re-exported so callers can name `CompositeAlphaMode`, `AdapterInfo` and the
/// rest without adding a wgpu dependency of their own — and, more importantly,
/// without adding a *different* one. Two wgpu versions in a tree means two
/// incompatible `Device` types.
pub use vello::wgpu;
pub use vello::{Scene, kurbo, peniko};

use std::num::NonZeroUsize;

use vello::{AaConfig, AaSupport, RenderParams, RendererOptions};

use surface::SurfaceRecord;

/// Anything that can go wrong that is worth telling the caller about.
#[derive(Debug)]
pub enum RenderError {
    /// No GPU and no software rasteriser. On a developer machine this usually
    /// means no Vulkan driver; `mesa-vulkan-drivers` supplies a software one.
    NoAdapter(String),
    Device(String),
    Surface(String),
    Vello(String),
    Readback(String),
    /// A surface id that this renderer does not know, or knows in the wrong
    /// mode — asking a swapchain surface for its pixels, say.
    NoSuchSurface(SurfaceId),
}

/// What to do about a missing adapter, for the backend this platform uses.
#[cfg(target_os = "windows")]
const NO_ADAPTER_HINT: &str = "uniwidged draws through Direct3D 12, which Windows 10 and later \
     provide; check `dxdiag`, and that `WGPU_BACKEND` is unset or names dx12";
#[cfg(target_os = "macos")]
const NO_ADAPTER_HINT: &str = "uniwidged draws through Metal, which every supported Mac has; \
     check that `WGPU_BACKEND` is unset or names metal";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const NO_ADAPTER_HINT: &str =
    "install a Vulkan driver (mesa-vulkan-drivers provides a software one)";

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::NoAdapter(why) => {
                write!(f, "no GPU or software adapter: {why} — {NO_ADAPTER_HINT}")
            }
            RenderError::Device(why) => write!(f, "device: {why}"),
            RenderError::Surface(why) => write!(f, "surface: {why}"),
            RenderError::Vello(why) => write!(f, "vello: {why}"),
            RenderError::Readback(why) => write!(f, "readback: {why}"),
            RenderError::NoSuchSurface(id) => write!(f, "no surface {id:?} in this renderer"),
        }
    }
}

impl std::error::Error for RenderError {}

/// What one frame actually cost.
///
/// Returned from [`Renderer::render`] so the claims in the module note are
/// measurable rather than asserted. The scheduler's soak test watches
/// `submits`: on an idle desktop it must be zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameStats {
    /// Surfaces that were dirty and got a new frame.
    pub rendered: usize,
    /// Surfaces that were clean and cost nothing.
    pub skipped: usize,
    /// `queue.submit` calls: one per rendered surface (Vello's own), plus one
    /// for the blits if anything was rendered at all.
    pub submits: usize,
}

/// The device, the surfaces and the one Vello renderer they share.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    vello: vello::Renderer,
    /// One blitter for every surface; it caches a pipeline per destination
    /// format and alpha conversion. See `blit.rs` for why this is not
    /// `wgpu::util::TextureBlitter`.
    blitter: blit::Blitter,
    surfaces: Vec<SurfaceRecord>,
    next_id: u32,
}

/// What this platform's swapchains need from the instance.
///
/// **Windows.** A widget window is created with `WS_EX_NOREDIRECTIONBITMAP`,
/// which is what lets DWM blend it per-pixel against the wallpaper — and also
/// what makes an ordinary HWND swapchain show *nothing at all*, because there
/// is no redirection surface for it to land in. wgpu's DX12 backend can instead
/// make a DirectComposition target for the window and a composition swapchain
/// on its visual (`DxgiFromVisual`), which is the path Ephemeris took by hand
/// and the only one that offers `PreMultiplied`. DX12 alone, because the other
/// backends would hand the same window an HWND swapchain and an opaque
/// rectangle. `WGPU_BACKEND` still overrides it.
///
/// Everywhere else the defaults are already right.
fn platform_instance(descriptor: wgpu::InstanceDescriptor) -> wgpu::InstanceDescriptor {
    // wgpu's own variables — `WGPU_BACKEND`, `WGPU_VALIDATION`,
    // `WGPU_DX12_COMPILER` and the rest — for somebody debugging a driver.
    #[cfg(target_os = "windows")]
    {
        let backend_chosen = wgpu::Backends::from_env().is_some();
        let mut descriptor = descriptor.with_env();
        if !backend_chosen {
            descriptor.backends = wgpu::Backends::DX12;
        }
        descriptor.backend_options.dx12.presentation_system =
            wgpu::Dx12SwapchainKind::DxgiFromVisual;
        descriptor
    }
    #[cfg(not(target_os = "windows"))]
    {
        descriptor.with_env()
    }
}

impl Renderer {
    /// A renderer with no surface yet.
    ///
    /// The adapter is chosen without a surface to be compatible with, which is
    /// right for headless use and fine on the overwhelmingly common desktop
    /// with one GPU. A host that will create swapchain surfaces should use
    /// [`Renderer::new_compatible_with`] so that the adapter is chosen against
    /// a real surface — on a hybrid-graphics laptop the two can differ.
    pub fn new() -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(platform_instance(
            wgpu::InstanceDescriptor::new_without_display_handle(),
        ));
        Self::build(instance, None)
    }

    /// A renderer whose adapter is chosen against a surface the caller already
    /// has, and which is returned registered as the first surface.
    ///
    /// # Safety
    ///
    /// The handles must outlive the renderer, and the surface must already
    /// have been configured by its window system and carry no buffer — see
    /// `ARCHITECTURE.md` §10.2 and the M0 spike.
    pub unsafe fn new_compatible_with(
        display: raw_window_handle::RawDisplayHandle,
        window: raw_window_handle::RawWindowHandle,
        width: u32,
        height: u32,
    ) -> Result<(Self, SurfaceId), RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            // wgpu 29 wants the display handle here, not only at surface
            // creation: Vulkan ignores it, but the GL backend needs it to bind
            // EGL to this display. Found in the M0 spike.
            ..wgpu::InstanceDescriptor::new_with_display_handle(Box::new(surface::DisplayHandle(
                display,
            )))
        });
        let wgpu_surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(display),
                raw_window_handle: window,
            })
        }
        .map_err(|e| RenderError::Surface(e.to_string()))?;

        let mut renderer = Self::build(instance, Some(&wgpu_surface))?;
        let id = renderer.register_swapchain(wgpu_surface, width, height)?;
        Ok((renderer, id))
    }

    fn build(
        instance: wgpu::Instance,
        compatible: Option<&wgpu::Surface<'static>>,
    ) -> Result<Self, RenderError> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            // A widget is a few thousand paths at 60 Hz at most. Waking the
            // discrete GPU for that is how a widget framework earns a
            // reputation for eating batteries.
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: compatible,
            force_fallback_adapter: false,
        }))
        .map_err(|e| RenderError::NoAdapter(e.to_string()))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("uniwidged"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| RenderError::Device(e.to_string()))?;

        let vello = vello::Renderer::new(
            &device,
            RendererOptions {
                use_cpu: false,
                antialiasing_support: AaSupport::area_only(),
                num_init_threads: NonZeroUsize::new(1),
                ..Default::default()
            },
        )
        .map_err(|e| RenderError::Vello(e.to_string()))?;

        let blitter = blit::Blitter::new(&device);

        Ok(Self {
            device,
            queue,
            instance,
            adapter,
            vello,
            blitter,
            surfaces: Vec::new(),
            next_id: 0,
        })
    }

    /// Which adapter was chosen, for the start-up log and `uniwidged doctor`.
    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.adapter.get_info()
    }

    fn take_id(&mut self) -> SurfaceId {
        let id = SurfaceId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Adds a surface the caller will read the pixels of rather than present.
    ///
    /// What X11 falls back to and what every headless test uses.
    pub fn add_readback_surface(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<SurfaceId, RenderError> {
        let id = self.take_id();
        let record = SurfaceRecord::new_readback(&self.device, id, width, height);
        self.surfaces.push(record);
        Ok(id)
    }

    /// Adds a surface presented through a wgpu swapchain.
    ///
    /// # Safety
    ///
    /// As [`Renderer::new_compatible_with`].
    pub unsafe fn add_swapchain_surface(
        &mut self,
        display: raw_window_handle::RawDisplayHandle,
        window: raw_window_handle::RawWindowHandle,
        width: u32,
        height: u32,
    ) -> Result<SurfaceId, RenderError> {
        let wgpu_surface = unsafe {
            self.instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(display),
                    raw_window_handle: window,
                })
        }
        .map_err(|e| RenderError::Surface(e.to_string()))?;
        self.register_swapchain(wgpu_surface, width, height)
    }

    fn register_swapchain(
        &mut self,
        wgpu_surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<SurfaceId, RenderError> {
        let id = self.take_id();
        let record = SurfaceRecord::new_swapchain(
            &self.device,
            &self.adapter,
            id,
            wgpu_surface,
            width,
            height,
        )?;
        self.surfaces.push(record);
        Ok(id)
    }

    pub fn remove_surface(&mut self, id: SurfaceId) {
        self.surfaces.retain(|s| s.id != id);
    }

    pub fn surface_count(&self) -> usize {
        self.surfaces.len()
    }

    /// How a surface reaches a screen.
    ///
    /// The host needs this: a readback surface's pixels have to be handed to
    /// the platform's own compositing path, and a swapchain surface's must not
    /// be touched.
    pub fn presentation(&self, id: SurfaceId) -> Option<Presentation> {
        self.surface(id).map(|s| s.presentation())
    }

    /// The swapchain format a surface negotiated, for the start-up log.
    pub fn swapchain_format(&self, id: SurfaceId) -> Option<wgpu::TextureFormat> {
        self.surface(id).and_then(|s| s.swapchain_format())
    }

    /// The alpha mode a swapchain surface ended up with.
    ///
    /// `Opaque` here means the platform refused per-pixel alpha and the widget
    /// will sit on a solid rectangle — worth saying out loud at start-up rather
    /// than leaving the user to notice.
    pub fn alpha_mode(&self, id: SurfaceId) -> Option<wgpu::CompositeAlphaMode> {
        self.surface(id)?.alpha_mode()
    }

    fn surface(&self, id: SurfaceId) -> Option<&SurfaceRecord> {
        self.surfaces.iter().find(|s| s.id == id)
    }

    fn surface_mut(&mut self, id: SurfaceId) -> Option<&mut SurfaceRecord> {
        self.surfaces.iter_mut().find(|s| s.id == id)
    }

    pub fn resize(&mut self, id: SurfaceId, width: u32, height: u32) -> Result<(), RenderError> {
        let device = &self.device;
        self.surfaces
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or(RenderError::NoSuchSurface(id))?
            .resize(device, width, height);
        Ok(())
    }

    pub fn size(&self, id: SurfaceId) -> Option<(u32, u32)> {
        self.surface(id).map(|s| s.size())
    }

    /// Marks a surface as needing a new frame.
    ///
    /// Called only when the widget's view tree actually changed — see
    /// `uniwidged_view::View::differs_from`. Everything about this crate's cost
    /// model depends on that being true.
    pub fn mark_dirty(&mut self, id: SurfaceId) {
        if let Some(s) = self.surface_mut(id) {
            s.dirty = true;
        }
    }

    pub fn mark_all_dirty(&mut self) {
        for s in &mut self.surfaces {
            s.dirty = true;
        }
    }

    /// Whether anything at all needs drawing.
    ///
    /// The scheduler checks this before doing any GPU work: with nothing dirty,
    /// a wake-up costs one comparison per surface and no driver call.
    pub fn has_work(&self) -> bool {
        self.surfaces.iter().any(|s| s.dirty)
    }

    /// Renders every dirty surface for which `scenes` supplies a scene.
    ///
    /// A surface that is dirty but has no scene this frame stays dirty: the
    /// widget's view has not been rebuilt yet, and dropping the dirty flag
    /// would lose the update entirely.
    pub fn render(&mut self, scenes: &[(SurfaceId, &Scene)]) -> Result<FrameStats, RenderError> {
        let mut stats = FrameStats {
            skipped: self.surfaces.iter().filter(|s| !s.dirty).count(),
            ..Default::default()
        };

        // 1. Rasterise. Vello submits its own compute pass per call; see the
        //    module note on why that cannot be batched here.
        let mut painted: Vec<SurfaceId> = Vec::new();
        for (id, scene) in scenes {
            let Some(record) = self.surfaces.iter().find(|s| s.id == *id) else {
                return Err(RenderError::NoSuchSurface(*id));
            };
            if !record.dirty {
                continue;
            }
            let (width, height) = record.size();
            let params = RenderParams {
                // Transparent, not a background colour: everywhere the scene
                // does not draw, the wallpaper must show through untouched.
                base_color: vello::peniko::Color::TRANSPARENT,
                width,
                height,
                antialiasing_method: AaConfig::Area,
            };
            self.vello
                .render_to_texture(
                    &self.device,
                    &self.queue,
                    scene,
                    record.target_view(),
                    &params,
                )
                .map_err(|e| RenderError::Vello(e.to_string()))?;
            stats.rendered += 1;
            stats.submits += 1;
            painted.push(*id);
        }

        if painted.is_empty() {
            return Ok(stats);
        }

        // 2. One encoder for every blit and copy in this frame, one submit.
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("uniwidged-frame"),
            });
        let mut presented: Vec<(SurfaceId, wgpu::SurfaceTexture)> = Vec::new();
        for id in &painted {
            let Some(record) = self.surfaces.iter().find(|s| s.id == *id) else {
                continue;
            };
            if let Some(frame) =
                record.encode_present(&self.device, &mut encoder, &mut self.blitter)?
            {
                presented.push((*id, frame));
            }
        }
        self.queue.submit(Some(encoder.finish()));
        stats.submits += 1;

        // 3. Present, and only now clear the dirty flags — a surface whose
        //    swapchain image could not be acquired stays dirty and is retried.
        for (id, frame) in presented {
            frame.present();
            if let Some(s) = self.surface_mut(id) {
                s.dirty = false;
            }
        }
        for id in painted {
            if let Some(s) = self.surface_mut(id)
                && s.is_readback()
            {
                s.dirty = false;
            }
        }

        Ok(stats)
    }

    /// The pixels of a readback surface, tightly packed RGBA8.
    ///
    /// Errors for a swapchain surface: those pixels belong to the compositor.
    pub fn read_pixels(&mut self, id: SurfaceId) -> Result<Image, RenderError> {
        let record = self
            .surfaces
            .iter()
            .find(|s| s.id == id)
            .ok_or(RenderError::NoSuchSurface(id))?;
        readback::read(&self.device, &self.queue, record)
    }

    /// Writes a readback surface to a PNG. What the golden-image suite uses.
    pub fn save_png(&mut self, id: SurfaceId, path: &std::path::Path) -> Result<(), RenderError> {
        self.read_pixels(id)?.write_png(path)
    }
}

/// Tightly packed RGBA8, top row first, with **straight** (non-premultiplied)
/// alpha.
///
/// Straight because that is what Vello produces: `fine.wgsl` divides the
/// accumulated colour by its alpha before writing the texture. It is also what
/// PNG stores, so [`Image::write_png`] passes the bytes through untouched.
///
/// It is *not* what a compositor wants. `wl_shm`'s ARGB8888,
/// DirectComposition and `CAMetalLayer` all expect premultiplied, so the
/// conversion happens in the blit on the way to a swapchain — see `blit.rs`,
/// and the note there on how this was found. A caller compositing these pixels
/// itself, as the X11 fallback will, has to premultiply them first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA8. See the type note.
    pub data: Vec<u8>,
}

impl Image {
    /// The pixel at `(x, y)` as straight-alpha `(r, g, b, a)`, or `None`
    /// outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> Option<(u8, u8, u8, u8)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = ((y * self.width + x) * 4) as usize;
        Some((
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ))
    }

    /// The same image with premultiplied alpha.
    ///
    /// What a compositor handed these pixels directly would expect — the X11
    /// readback path, and anything else doing its own `PictOpOver`.
    pub fn to_premultiplied(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.data.len());
        for px in self.data.as_chunks::<4>().0 {
            let a = px[3] as u32;
            for c in &px[..3] {
                // Round to nearest rather than truncating: over a whole image
                // truncation is a visible darkening, not a rounding error.
                out.push(((*c as u32 * a + 127) / 255) as u8);
            }
            out.push(px[3]);
        }
        out
    }

    /// Writes a PNG. The data is already straight alpha, which is what the
    /// format stores, so nothing is converted.
    pub fn write_png(&self, path: &std::path::Path) -> Result<(), RenderError> {
        let file = std::fs::File::create(path)
            .map_err(|e| RenderError::Readback(format!("{}: {e}", path.display())))?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .and_then(|mut w| w.write_image_data(&self.data))
            .map_err(|e| RenderError::Readback(e.to_string()))
    }
}

#[cfg(test)]
mod tests;
