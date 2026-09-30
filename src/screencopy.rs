//! Screen capture for streaming and screenshots: `wlr-screencopy-unstable-v1`.
//!
//! This is the protocol `xdg-desktop-portal-wlr` speaks, so OBS, Discord, browsers and `grim`
//! capture monitors through it. GPU buffers (dmabuf) are rendered into directly, without a
//! trip through the CPU; shared-memory buffers take one GPU read-back.
use std::sync::atomic::{AtomicBool, Ordering};

use smithay::{
    backend::{
        allocator::{Buffer as _, Fourcc},
        renderer::{
            Bind, ExportMem, Offscreen, damage::OutputDamageTracker, gles::GlesRenderer,
        },
    },
    output::Output,
    reexports::{
        wayland_protocols_wlr::screencopy::v1::server::{
            zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
            zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
            protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_shm},
        },
    },
    utils::{Buffer as BufferCoords, Clock, Monotonic, Physical, Rectangle, Size, Transform},
    wayland::{
        dmabuf::get_dmabuf,
        shm::{BufferData, with_buffer_contents_mut},
    },
};

use crate::{State, render::OutputElement};

/// What a client asked to capture; fixed when the frame object is created.
pub struct FrameData {
    output: Output,
    overlay_cursor: bool,
    /// The captured rectangle in pixels of the output as displayed (after rotation).
    region: Rectangle<i32, Physical>,
    /// The whole output was requested, so GPU buffers can be rendered into directly.
    full: bool,
    used: AtomicBool,
}

/// A `copy` request waiting for the next frame of its output.
pub struct PendingCopy {
    frame: ZwlrScreencopyFrameV1,
    output: Output,
    region: Rectangle<i32, Physical>,
    overlay_cursor: bool,
    buffer: WlBuffer,
    /// `copy_with_damage`: wait until something on the output changed.
    wait_for_damage: bool,
}

impl State {
    pub fn create_screencopy_global(display: &DisplayHandle) -> smithay::reexports::wayland_server::backend::GlobalId {
        display.create_global::<State, ZwlrScreencopyManagerV1, ()>(3, ())
    }

    /// Size in pixels of `output` as displayed, and the transform its capture is rendered with.
    fn capture_geometry(&self, output: &Output) -> Option<(Size<i32, Physical>, Size<i32, Physical>, Transform)> {
        let mode = output.current_mode()?;
        // The nested window renders upside down internally (see `winit.rs`); real outputs do not.
        let transform = if self.udev.is_some() { output.current_transform() } else { Transform::Normal };
        Some((mode.size, transform.transform_size(mode.size), transform))
    }

    /// Answer requests that came in since the last frame using the elements just drawn.
    pub fn fulfill_screencopy(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
        elements: &[OutputElement],
        had_damage: bool,
    ) {
        if self.pending_copies.is_empty() {
            return;
        }
        let (ready, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_copies)
            .into_iter()
            .partition(|p| &p.output == output && (!p.wait_for_damage || had_damage));
        self.pending_copies = waiting;
        let clear = self.clear_color();
        for pending in ready {
            if !pending.frame.is_alive() {
                continue;
            }
            match self.capture(renderer, elements, clear, &pending) {
                Ok(()) => {
                    let now: std::time::Duration = Clock::<Monotonic>::new().now().into();
                    pending.frame.flags(zwlr_screencopy_frame_v1::Flags::empty());
                    pending.frame.ready((now.as_secs() >> 32) as u32, now.as_secs() as u32, now.subsec_nanos());
                }
                Err(error) => {
                    tracing::warn!("screen capture of {} failed: {error}", pending.output.name());
                    pending.frame.failed();
                }
            }
        }
    }

    fn capture(
        &self,
        renderer: &mut GlesRenderer,
        elements: &[OutputElement],
        clear: [f32; 4],
        pending: &PendingCopy,
    ) -> Result<(), String> {
        let (mode_size, size, transform) = self.capture_geometry(&pending.output).ok_or("the output has no mode")?;
        let scale = pending.output.current_scale().fractional_scale();
        let mut tracker = OutputDamageTracker::new(mode_size, scale, transform);
        // Without the cursor overlay, leave the cursor elements out.
        let visible: Vec<&OutputElement> = elements
            .iter()
            .filter(|e| pending.overlay_cursor || !matches!(e, OutputElement::Cursor(_)))
            .collect();

        if let Ok(dmabuf) = get_dmabuf(&pending.buffer) {
            let mut dmabuf = dmabuf.clone();
            let mut target = renderer.bind(&mut dmabuf).map_err(|e| format!("bind: {e}"))?;
            let result = tracker
                .render_output(renderer, &mut target, 0, &visible, clear)
                .map_err(|e| format!("render: {e:?}"))?;
            // The client reads the buffer as soon as it gets `ready`.
            let _ = result.sync.wait();
            return Ok(());
        }

        // Shared memory: render into a texture, read it back and copy the wanted part.
        let buffer_size: Size<i32, BufferCoords> = (size.w, size.h).into();
        let mut texture = Offscreen::<smithay::backend::renderer::gles::GlesTexture>::create_buffer(
            renderer,
            Fourcc::Argb8888,
            buffer_size,
        )
        .map_err(|e| format!("offscreen buffer: {e}"))?;
        let mut target = renderer.bind(&mut texture).map_err(|e| format!("bind: {e}"))?;
        tracker
            .render_output(renderer, &mut target, 0, &visible, clear)
            .map_err(|e| format!("render: {e:?}"))?;
        let region = Rectangle::<i32, BufferCoords>::new(
            (pending.region.loc.x, pending.region.loc.y).into(),
            (pending.region.size.w, pending.region.size.h).into(),
        );
        let mapping = renderer.copy_framebuffer(&target, region, Fourcc::Argb8888).map_err(|e| format!("read back: {e}"))?;
        let pixels = renderer.map_texture(&mapping).map_err(|e| format!("map: {e}"))?;
        let row = region.size.w as usize * 4;
        with_buffer_contents_mut(&pending.buffer, |ptr, len, data: BufferData| {
            let (offset, stride) = (data.offset as usize, data.stride as usize);
            for y in 0..region.size.h as usize {
                let start = offset + y * stride;
                if start + row > len || (y + 1) * row > pixels.len() {
                    return Err("the client's buffer is smaller than announced");
                }
                // SAFETY: bounds were checked above and the pool stays mapped during the callback.
                unsafe { std::ptr::copy_nonoverlapping(pixels[y * row..].as_ptr(), ptr.add(start), row) };
            }
            Ok(())
        })
        .map_err(|e| format!("shm access: {e:?}"))?
        .map_err(String::from)
    }

    fn screencopy_request(&mut self, frame: &ZwlrScreencopyFrameV1, data: &FrameData, buffer: WlBuffer, wait_for_damage: bool) {
        if data.used.swap(true, Ordering::Relaxed) {
            frame.post_error(zwlr_screencopy_frame_v1::Error::AlreadyUsed, "the frame was already copied");
            return;
        }
        let want = data.region.size;
        let acceptable = |format: u32| format == Fourcc::Argb8888 as u32 || format == Fourcc::Xrgb8888 as u32;
        let valid = if let Ok(dmabuf) = get_dmabuf(&buffer) {
            data.full && acceptable(dmabuf.format().code as u32) && dmabuf.size().w == want.w && dmabuf.size().h == want.h
        } else {
            with_buffer_contents_mut(&buffer, |_, _, d| {
                d.width == want.w
                    && d.height == want.h
                    && d.stride >= want.w * 4
                    && matches!(d.format, wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888)
            })
            .unwrap_or(false)
        };
        if !valid {
            frame.post_error(zwlr_screencopy_frame_v1::Error::InvalidBuffer, "the buffer does not match the announced format");
            return;
        }
        self.pending_copies.push(PendingCopy {
            frame: frame.clone(),
            output: data.output.clone(),
            region: data.region,
            overlay_cursor: data.overlay_cursor,
            buffer,
            wait_for_damage,
        });
        self.queue_redraw_all();
    }

    /// An output went away: capture requests waiting on it cannot be answered.
    pub fn fail_screencopies_of(&mut self, output: &Output) {
        self.pending_copies.retain(|p| {
            let keep = &p.output != output;
            if !keep {
                p.frame.failed();
            }
            keep
        });
    }
}

impl GlobalDispatch<ZwlrScreencopyManagerV1, (), State> for State {
    fn bind(
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _data: &(),
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, (), State> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        _manager: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        let (frame, overlay_cursor, output, region) = match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput { frame, overlay_cursor, output } => {
                (frame, overlay_cursor != 0, output, None)
            }
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion { frame, overlay_cursor, output, x, y, width, height } => {
                (frame, overlay_cursor != 0, output, Some((x, y, width, height)))
            }
            _ => return,
        };
        capture_output(state, data_init, frame, overlay_cursor, &output, region);
    }
}

fn capture_output(
    state: &mut State,
    data_init: &mut DataInit<'_, State>,
    frame: New<ZwlrScreencopyFrameV1>,
    overlay_cursor: bool,
    output: &WlOutput,
    region: Option<(i32, i32, i32, i32)>,
) {
    let Some(output) = Output::from_resource(output) else {
        let frame = data_init.init(frame, dead_frame());
        frame.failed();
        return;
    };
    let Some((_, size, _)) = state.capture_geometry(&output) else {
        let frame = data_init.init(frame, dead_frame());
        frame.failed();
        return;
    };
    // The region is given in logical coordinates of the output; the buffer is in pixels.
    let scale = output.current_scale().fractional_scale();
    let (region, full) = match region {
        None => (Rectangle::from_size(size), true),
        Some((x, y, w, h)) => {
            let rect = Rectangle::<i32, Physical>::new(
                ((f64::from(x) * scale).round() as i32, (f64::from(y) * scale).round() as i32).into(),
                ((f64::from(w) * scale).round() as i32, (f64::from(h) * scale).round() as i32).into(),
            );
            let clipped = rect.intersection(Rectangle::from_size(size));
            match clipped {
                Some(clipped) if w > 0 && h > 0 => (clipped, clipped == Rectangle::from_size(size)),
                _ => {
                    let frame = data_init.init(frame, dead_frame());
                    frame.failed();
                    return;
                }
            }
        }
    };
    let resource = data_init.init(
        frame,
        FrameData { output, overlay_cursor, region, full, used: AtomicBool::new(false) },
    );
    let (w, h) = (region.size.w as u32, region.size.h as u32);
    resource.buffer(wl_shm::Format::Argb8888, w, h, w * 4);
    // Version 3 announces all buffer types first and then says it is done; older clients
    // simply take the shared-memory one.
    if resource.version() >= 3 {
        if state.udev.is_some() && full {
            // GPU buffers only where a GPU renders for us and the whole output is wanted.
            resource.linux_dmabuf(Fourcc::Argb8888 as u32, w, h);
        }
        resource.buffer_done();
    }
}

/// Data for a frame that fails immediately (unknown output).
fn dead_frame() -> FrameData {
    FrameData {
        output: Output::new(
            String::new(),
            smithay::output::PhysicalProperties {
                size: (0, 0).into(),
                subpixel: smithay::output::Subpixel::Unknown,
                make: String::new(),
                model: String::new(),
            },
        ),
        overlay_cursor: false,
        region: Rectangle::default(),
        full: false,
        used: AtomicBool::new(true),
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, FrameData, State> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        data: &FrameData,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => state.screencopy_request(frame, data, buffer, false),
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => {
                state.screencopy_request(frame, data, buffer, true)
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut State,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        frame: &ZwlrScreencopyFrameV1,
        _data: &FrameData,
    ) {
        state.pending_copies.retain(|p| &p.frame != frame);
    }
}
