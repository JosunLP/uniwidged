//! One widget's surface: where it is rasterised, and how it reaches a screen.

use vello::wgpu;

use crate::RenderError;
use crate::blit::{AlphaConversion, Blitter};

/// A surface registered with the renderer, addressing one widget's output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SurfaceId(pub u32);

/// How a surface's pixels reach a screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presentation {
    /// Handed to the compositor through a wgpu swapchain.
    Swapchain,
    /// Read back and composited by the caller. X11 without a transparent
    /// swapchain, and every headless test.
    Readback,
}

/// Lends a display handle to wgpu's instance descriptor, which wants an owned
/// `HasDisplayHandle` rather than a raw one.
#[derive(Debug)]
pub(crate) struct DisplayHandle(pub(crate) raw_window_handle::RawDisplayHandle);

// SAFETY: a display pointer is valid on any thread, and the connection that
// owns it outlives the renderer — the caller guarantees that at the `unsafe`
// constructor.
unsafe impl Send for DisplayHandle {}
unsafe impl Sync for DisplayHandle {}

impl raw_window_handle::HasDisplayHandle for DisplayHandle {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        // SAFETY: as above.
        Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(self.0) })
    }
}

enum Mode {
    Swapchain {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    },
    Readback {
        /// Sized for 256-byte-aligned rows, which is what
        /// `copy_texture_to_buffer` requires.
        buffer: wgpu::Buffer,
        padded_bytes_per_row: u32,
    },
}

pub(crate) struct SurfaceRecord {
    pub(crate) id: SurfaceId,
    pub(crate) dirty: bool,
    width: u32,
    height: u32,
    /// Vello writes here; the blit or the copy reads from here.
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    mode: Mode,
}

/// Every usage the intermediate needs, in one place so the two constructors
/// cannot drift: STORAGE so Vello can write it, TEXTURE_BINDING so the blitter
/// can sample it, COPY_SRC so readback can copy it.
const TARGET_USAGE: wgpu::TextureUsages = wgpu::TextureUsages::STORAGE_BINDING
    .union(wgpu::TextureUsages::TEXTURE_BINDING)
    .union(wgpu::TextureUsages::COPY_SRC);

fn create_target(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("uniwidged-target"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: TARGET_USAGE,
        view_formats: &[],
    })
}

/// Rows in a `copy_texture_to_buffer` destination must start on a 256-byte
/// boundary, so a 100-pixel-wide image is copied 512 bytes to the row and
/// unpacked afterwards.
#[cfg(test)]
pub(crate) fn padded_row_for_test(width: u32) -> u32 {
    padded_row(width)
}

fn padded_row(width: u32) -> u32 {
    let unpadded = width * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    unpadded.div_ceil(align) * align
}

fn create_readback_buffer(device: &wgpu::Device, width: u32, height: u32) -> (wgpu::Buffer, u32) {
    let padded = padded_row(width.max(1));
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("uniwidged-readback"),
        size: (padded * height.max(1)) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    (buffer, padded)
}

impl SurfaceRecord {
    pub(crate) fn new_readback(
        device: &wgpu::Device,
        id: SurfaceId,
        width: u32,
        height: u32,
    ) -> Self {
        let target = create_target(device, width, height);
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let (buffer, padded_bytes_per_row) = create_readback_buffer(device, width, height);
        Self {
            id,
            // A brand-new surface has never been drawn, so it is dirty by
            // definition — otherwise the first frame never happens.
            dirty: true,
            width: width.max(1),
            height: height.max(1),
            target,
            target_view,
            mode: Mode::Readback {
                buffer,
                padded_bytes_per_row,
            },
        }
    }

    pub(crate) fn new_swapchain(
        device: &wgpu::Device,
        adapter: &wgpu::Adapter,
        id: SurfaceId,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        let caps = surface.get_capabilities(adapter);
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
            .ok_or_else(|| RenderError::Surface("the surface supports no format".into()))?;

        // Per-pixel alpha is what makes this a widget rather than a window on a
        // grey rectangle, so it is preferred wherever the platform offers it.
        // Where it does not, the surface still works and the caller is told —
        // see `Renderer::alpha_mode`.
        let alpha_mode = [
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::Inherit,
            wgpu::CompositeAlphaMode::PostMultiplied,
        ]
        .into_iter()
        .find(|m| caps.alpha_modes.contains(m))
        .unwrap_or(wgpu::CompositeAlphaMode::Auto);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };
        surface.configure(device, &config);

        let target = create_target(device, width, height);
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        Ok(Self {
            id,
            dirty: true,
            width: width.max(1),
            height: height.max(1),
            target,
            target_view,
            mode: Mode::Swapchain { surface, config },
        })
    }

    pub(crate) fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub(crate) fn target_view(&self) -> &wgpu::TextureView {
        &self.target_view
    }

    pub(crate) fn is_readback(&self) -> bool {
        matches!(self.mode, Mode::Readback { .. })
    }

    pub(crate) fn presentation(&self) -> Presentation {
        match self.mode {
            Mode::Swapchain { .. } => Presentation::Swapchain,
            Mode::Readback { .. } => Presentation::Readback,
        }
    }

    pub(crate) fn swapchain_format(&self) -> Option<wgpu::TextureFormat> {
        match &self.mode {
            Mode::Swapchain { config, .. } => Some(config.format),
            Mode::Readback { .. } => None,
        }
    }

    pub(crate) fn alpha_mode(&self) -> Option<wgpu::CompositeAlphaMode> {
        match &self.mode {
            Mode::Swapchain { config, .. } => Some(config.alpha_mode),
            Mode::Readback { .. } => None,
        }
    }

    pub(crate) fn readback_parts(&self) -> Option<(&wgpu::Texture, &wgpu::Buffer, u32)> {
        match &self.mode {
            Mode::Readback {
                buffer,
                padded_bytes_per_row,
            } => Some((&self.target, buffer, *padded_bytes_per_row)),
            Mode::Swapchain { .. } => None,
        }
    }

    pub(crate) fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == (self.width, self.height) {
            return;
        }
        self.width = width;
        self.height = height;
        self.target = create_target(device, width, height);
        self.target_view = self
            .target
            .create_view(&wgpu::TextureViewDescriptor::default());
        match &mut self.mode {
            Mode::Swapchain { surface, config } => {
                config.width = width;
                config.height = height;
                surface.configure(device, config);
            }
            Mode::Readback {
                buffer,
                padded_bytes_per_row,
            } => {
                let (new_buffer, padded) = create_readback_buffer(device, width, height);
                *buffer = new_buffer;
                *padded_bytes_per_row = padded;
            }
        }
        // The old frame is the wrong size now.
        self.dirty = true;
    }

    /// Records this surface's presentation onto the frame's shared encoder.
    ///
    /// Returns the acquired swapchain image, which the caller presents *after*
    /// the submit — presenting before it would show an unfinished frame.
    pub(crate) fn encode_present(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        blitter: &mut Blitter,
    ) -> Result<Option<wgpu::SurfaceTexture>, RenderError> {
        match &self.mode {
            Mode::Swapchain { surface, config } => {
                let frame = match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(f)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
                    // Transient. Reconfigure and skip: the surface stays dirty
                    // and is retried on the next frame, which is better than
                    // failing a whole desktop's frame for one widget.
                    wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                        surface.configure(device, config);
                        return Ok(None);
                    }
                    other => {
                        return Err(RenderError::Surface(format!(
                            "could not acquire a swapchain image: {other:?}"
                        )));
                    }
                };
                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                blitter.blit(
                    device,
                    encoder,
                    &self.target_view,
                    &view,
                    config.format,
                    AlphaConversion::for_mode(config.alpha_mode),
                );
                Ok(Some(frame))
            }
            Mode::Readback {
                buffer,
                padded_bytes_per_row,
            } => {
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.target,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(*padded_bytes_per_row),
                            rows_per_image: None,
                        },
                    },
                    wgpu::Extent3d {
                        width: self.width,
                        height: self.height,
                        depth_or_array_layers: 1,
                    },
                );
                Ok(None)
            }
        }
    }
}
