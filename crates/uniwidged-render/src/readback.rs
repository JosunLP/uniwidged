//! Getting pixels back off the GPU.
//!
//! Used by the X11 presentation fallback and by every golden-image test. The
//! rows come back 256-byte aligned because that is what
//! `copy_texture_to_buffer` requires, and are unpacked here so that callers see
//! a tightly packed image and nothing else has to know about the alignment.

use vello::wgpu;

use crate::surface::SurfaceRecord;
use crate::{Image, RenderError};

pub(crate) fn read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    record: &SurfaceRecord,
) -> Result<Image, RenderError> {
    let Some((_, buffer, padded)) = record.readback_parts() else {
        return Err(RenderError::Readback(format!(
            "surface {:?} is presented through a swapchain — those pixels belong \
             to the compositor, not to us",
            record.id
        )));
    };
    let (width, height) = record.size();

    // The copy was recorded onto the frame's encoder and submitted with it, so
    // there is nothing to submit here — only to wait for.
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    // Polling can fail on device loss, and that has to surface as an error
    // rather than as a panic inside `get_mapped_range` further down.
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| RenderError::Readback(format!("device poll failed: {e}")))?;
    let _ = queue;
    match rx.recv() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(RenderError::Readback(e.to_string())),
        Err(_) => return Err(RenderError::Readback("the map callback was dropped".into())),
    }

    let unpadded = width as usize * 4;
    let mut data = Vec::with_capacity(unpadded * height as usize);
    {
        let mapped = slice.get_mapped_range();
        for row in 0..height as usize {
            let start = row * padded as usize;
            data.extend_from_slice(&mapped[start..start + unpadded]);
        }
    }
    buffer.unmap();

    Ok(Image {
        width,
        height,
        data,
    })
}
