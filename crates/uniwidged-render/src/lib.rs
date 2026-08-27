//! One device, many surfaces, one submit.
//!
//! This crate is the reason uniwidged is a single process rather than one
//! process per widget. A wgpu device and its driver context cost roughly
//! 30–60 MB and a non-trivial start-up; paying that once for eight widgets is
//! the difference between a widget framework and eight applications pretending
//! to be one.
//!
//! ## The frame
//!
//! Each surface renders through Vello's compute rasterizer into its own
//! intermediate storage texture, which is then blitted onto that surface's
//! swapchain image. The indirection is not optional: Vello writes from a
//! compute shader into a storage texture, and swapchain images are generally
//! not valid storage targets.
//!
//! Every dirty surface is encoded onto **one** `CommandEncoder` and finished
//! with **one** `queue.submit`. Clean surfaces are not touched at all — they
//! keep the frame they already have, at no cost.
//!
//! ## The CPU fallback
//!
//! `uniwidged-compose` produces a `vello::Scene`, not wgpu calls, and that is
//! deliberate: the same scene can go through `vello_cpu` instead. Where a
//! platform cannot give a GPU swapchain per-pixel alpha — X11 is the live
//! case, see `ARCHITECTURE.md` §10.1 — the widget still renders, from the same
//! scene, to pixel-identical output. The golden images stay valid across both
//! paths, which is what makes the fallback trustworthy rather than a second
//! renderer nobody looks at.
//!
//! ## Status
//!
//! Skeleton. Milestone M1 in `ROADMAP.md`, after the layer-shell spike proves
//! the surface seam.

#![allow(dead_code)]

use uniwidged_core::geom::PhysicalRect;

/// A surface registered with the renderer. Handed out by [`Renderer`] and used
/// by the host to address one widget's swapchain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfaceId(pub u32);

/// Which rasterizer a surface is running through.
///
/// Chosen per surface rather than per process, because a machine can plausibly
/// have a Wayland output that supports a transparent GPU swapchain and an X11
/// one that does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Vello on wgpu. The default everywhere it works.
    Gpu,
    /// `vello_cpu`, from the same scene. See the module note.
    Cpu,
}

/// The device, the surfaces and the per-surface renderers.
#[derive(Debug, Default)]
pub struct Renderer {
    surfaces: Vec<SurfaceRecord>,
}

#[derive(Debug)]
struct SurfaceRecord {
    id: SurfaceId,
    size: PhysicalRect,
    backend: Backend,
    dirty: bool,
}

impl Renderer {
    /// Marks a surface as needing a new frame.
    ///
    /// Called only when [`uniwidged_view::View::differs_from`] said the tree
    /// changed. A surface that is not dirty is not encoded, not submitted and
    /// not presented.
    pub fn mark_dirty(&mut self, id: SurfaceId) {
        if let Some(s) = self.surfaces.iter_mut().find(|s| s.id == id) {
            s.dirty = true;
        }
    }

    /// Whether anything at all needs drawing.
    ///
    /// The scheduler checks this before doing any GPU work: with nothing
    /// dirty, a wake-up costs one comparison.
    pub fn has_work(&self) -> bool {
        self.surfaces.iter().any(|s| s.dirty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_renderer_with_nothing_dirty_has_no_work() {
        // The check the scheduler makes on every wake. If this is ever true
        // for an idle desktop, the process is submitting idle frames.
        assert!(!Renderer::default().has_work());
    }
}
