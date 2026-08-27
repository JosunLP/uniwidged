//! No window system at all.
//!
//! Not a courtesy backend. Every visual test in CI runs through this one: a
//! surface that renders to a texture and reads back a PNG is what makes
//! golden-image snapshots possible on a machine with no GPU and no display.
//! The path through `uniwidged-compose` is byte-for-byte the same one that
//! ships, which is the only reason a headless snapshot is evidence of
//! anything.
//!
//! It is also what `uniwidged doctor` uses to prove the stack works before
//! blaming the compositor.

use crate::{Capabilities, Placement};
use uniwidged_core::geom::PhysicalRect;

#[derive(Debug)]
pub struct HeadlessShell {
    surfaces: Vec<Placement>,
}

impl HeadlessShell {
    pub fn new() -> Self {
        Self {
            surfaces: Vec::new(),
        }
    }

    /// Everything a headless surface can honestly claim.
    ///
    /// `per_pixel_alpha` is true because the readback really does carry an
    /// alpha channel; `gpu_swapchain` is false because there is no swapchain
    /// to have — the renderer draws to a texture either way.
    pub fn capabilities(&self) -> Capabilities {
        Capabilities {
            below_windows: true,
            self_placing: true,
            per_pixel_alpha: true,
            gpu_swapchain: false,
            backdrop_blur: false,
        }
    }

    pub fn create_surface(&mut self, rect: PhysicalRect, scale: f64) -> usize {
        self.surfaces.push(Placement {
            rect,
            output: None,
            scale,
        });
        self.surfaces.len() - 1
    }
}

impl Default for HeadlessShell {
    fn default() -> Self {
        Self::new()
    }
}
