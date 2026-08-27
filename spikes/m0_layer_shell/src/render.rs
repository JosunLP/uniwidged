//! The GPU half: a wgpu swapchain on a surface somebody else configured, and
//! one Vello scene on it.

use std::num::NonZeroUsize;

use vello::kurbo::{Affine, Point, Rect, RoundedRect, Stroke};
use vello::peniko::{Color, ColorStop, Extend, Gradient, color::AlphaColor};
use vello::wgpu;
use vello::{AaConfig, AaSupport, RenderParams, RendererOptions, Scene};

/// What the surface turned out to support. The point of the spike.
#[derive(Debug)]
pub struct SurfaceReport {
    pub adapter: String,
    pub backend: String,
    pub format: wgpu::TextureFormat,
    pub alpha_modes: Vec<wgpu::CompositeAlphaMode>,
    pub chosen_alpha: wgpu::CompositeAlphaMode,
}

/// Lends the compositor's display handle to wgpu.
///
/// Needed because the instance descriptor takes an owned `HasDisplayHandle`
/// rather than a raw handle.
#[derive(Debug)]
struct DisplayHandle(raw_window_handle::RawDisplayHandle);

// SAFETY: a `wl_display` pointer is valid on any thread; the connection that
// owns it outlives the instance for the whole of this program.
unsafe impl Send for DisplayHandle {}
unsafe impl Sync for DisplayHandle {}

impl raw_window_handle::HasDisplayHandle for DisplayHandle {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        // SAFETY: as above — the handle outlives this borrow.
        Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(self.0) })
    }
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    vello: vello::Renderer,
    target: wgpu::TextureView,
    blitter: wgpu::util::TextureBlitter,
    pub report: SurfaceReport,
}

impl Renderer {
    /// # Safety
    ///
    /// The handles must outlive the returned renderer, and the surface must
    /// already have been configured by its compositor and carry no buffer.
    pub unsafe fn new(
        display: raw_window_handle::RawDisplayHandle,
        window: raw_window_handle::RawWindowHandle,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        // wgpu 29 wants the display handle at *instance* creation, not only at
        // surface creation. Vulkan ignores it, but the GL backend needs it to
        // bind EGL to this `wl_display` — and GL is the fallback that the X11
        // transparency mitigation (ARCHITECTURE.md §10.1) is built on, so it is
        // worth wiring up properly here rather than discovering it later.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            ..wgpu::InstanceDescriptor::new_with_display_handle(Box::new(DisplayHandle(display)))
        });
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(display),
                raw_window_handle: window,
            })
        }
        .map_err(|e| format!("wgpu would not take the Wayland surface: {e}"))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .map_err(|e| format!("no adapter can draw on this surface: {e}"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("uniwidged-spike"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| format!("no device: {e}"))?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
                )
            })
            .or_else(|| caps.formats.first().copied())
            .ok_or("the surface supports no texture format at all")?;

        // The whole question, in one expression. `PreMultiplied` is what makes
        // the wallpaper visible through the panel; `Opaque` is a widget on a
        // grey rectangle. See ARCHITECTURE.md §10.1.
        let chosen_alpha = if caps
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
        {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else if caps
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::Inherit)
        {
            wgpu::CompositeAlphaMode::Inherit
        } else {
            caps.alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto)
        };

        let adapter_info = adapter.get_info();
        let report = SurfaceReport {
            adapter: adapter_info.name.clone(),
            backend: format!("{:?}", adapter_info.backend),
            format,
            alpha_modes: caps.alpha_modes.clone(),
            chosen_alpha,
        };

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: chosen_alpha,
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let vello = vello::Renderer::new(
            &device,
            RendererOptions {
                use_cpu: false,
                antialiasing_support: AaSupport::area_only(),
                num_init_threads: NonZeroUsize::new(1),
                ..Default::default()
            },
        )
        .map_err(|e| format!("Vello would not start: {e}"))?;

        let target = intermediate(&device, config.width, config.height);
        let blitter = wgpu::util::TextureBlitter::new(&device, format);

        Ok(Self {
            device,
            queue,
            surface,
            config,
            vello,
            target,
            blitter,
            report,
        })
    }

    pub fn render(&mut self, scene: &Scene) -> Result<(), String> {
        // `TRANSPARENT`, not a background colour. Everywhere the scene does not
        // draw, the wallpaper must show through untouched — that is what makes
        // this a widget rather than a window.
        let params = RenderParams {
            base_color: Color::TRANSPARENT,
            width: self.config.width,
            height: self.config.height,
            antialiasing_method: AaConfig::Area,
        };
        self.vello
            .render_to_texture(&self.device, &self.queue, scene, &self.target, &params)
            .map_err(|e| format!("Vello render failed: {e}"))?;

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            other => return Err(format!("no swapchain image: {other:?}")),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("spike-blit"),
            });
        self.blitter
            .copy(&self.device, &mut encoder, &self.target, &view);
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}

fn intermediate(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("spike-intermediate"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

/// The panel, as a widget would look.
///
/// Every element here is one of the quality claims in `ARCHITECTURE.md` §7,
/// made concrete: a real Gaussian shadow rather than stacked strokes, a
/// gradient interpolated in premultiplied alpha, a sub-pixel hairline that
/// stays a hairline, and rounded caps that are actually round.
pub fn widget_scene(width: f64, height: f64) -> Scene {
    let mut scene = Scene::new();
    if std::env::var_os("SPIKE_FORCE_TOPLEVEL").is_some() {
        // An ordinary application window: fully opaque, edge to edge. Used to
        // prove the stacking order — if the layer surface really is on the
        // bottom layer, this covers it completely.
        scene.fill(
            vello::peniko::Fill::NonZero,
            Affine::IDENTITY,
            AlphaColor::from_rgba8(240, 90, 30, 255),
            None,
            &Rect::new(0.0, 0.0, width, height),
        );
        return scene;
    }
    let inset = 24.0;
    let body = RoundedRect::new(inset, inset, width - inset, height - inset, 18.0);

    // A real Gaussian blur. The shadow is what tells the eye the panel is
    // floating above the wallpaper rather than painted onto it.
    scene.draw_blurred_rounded_rect(
        Affine::translate((0.0, 6.0)),
        body.rect(),
        AlphaColor::from_rgba8(0, 0, 0, 110),
        18.0,
        14.0,
    );

    // The glass. Partly transparent on purpose: if the alpha never reaches the
    // compositor, this reads as flat grey and the spike has failed.
    let glass = Gradient::new_linear(Point::new(0.0, inset), Point::new(0.0, height - inset))
        .with_stops([
            ColorStop {
                offset: 0.0,
                color: AlphaColor::from_rgba8(38, 40, 48, 190).into(),
            },
            ColorStop {
                offset: 1.0,
                color: AlphaColor::from_rgba8(20, 21, 26, 150).into(),
            },
        ])
        .with_extend(Extend::Pad);
    scene.fill(
        vello::peniko::Fill::NonZero,
        Affine::IDENTITY,
        &glass,
        None,
        &body,
    );

    // A one-pixel specular highlight along the top edge. At any scale factor
    // this has to stay one pixel and stay crisp.
    scene.stroke(
        &Stroke::new(1.0),
        Affine::IDENTITY,
        AlphaColor::from_rgba8(255, 255, 255, 46),
        None,
        &body,
    );

    // Three meters, the shape a real widget takes.
    let track_x = inset + 22.0;
    let track_w = width - inset * 2.0 - 44.0;
    for (i, (value, color)) in [
        (0.32f64, AlphaColor::from_rgba8(120, 190, 255, 255)),
        (0.61, AlphaColor::from_rgba8(120, 190, 255, 255)),
        (0.94, AlphaColor::from_rgba8(255, 110, 110, 255)),
    ]
    .into_iter()
    .enumerate()
    {
        let y = inset + 46.0 + i as f64 * 30.0;
        let track = RoundedRect::new(track_x, y, track_x + track_w, y + 8.0, 4.0);
        scene.fill(
            vello::peniko::Fill::NonZero,
            Affine::IDENTITY,
            AlphaColor::from_rgba8(255, 255, 255, 28),
            None,
            &track,
        );
        let filled = RoundedRect::new(track_x, y, track_x + track_w * value, y + 8.0, 4.0);
        scene.fill(
            vello::peniko::Fill::NonZero,
            Affine::IDENTITY,
            color,
            None,
            &filled,
        );
    }

    // A title bar stand-in. Text needs a font stack, which is M2's problem —
    // this is a placeholder shaped like one so the layout reads correctly.
    scene.fill(
        vello::peniko::Fill::NonZero,
        Affine::IDENTITY,
        AlphaColor::from_rgba8(255, 255, 255, 200),
        None,
        &Rect::new(track_x, inset + 22.0, track_x + 74.0, inset + 32.0),
    );

    scene
}
