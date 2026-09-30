//! Captures one frame of an output or of the first toplevel through `ext-image-copy-capture`
//! and prints `WxH center=RRGGBB` (or fails). Usage: `mywm-capture-test output|toplevel [shm]`.
use std::{fs::File, os::fd::AsFd, os::unix::fs::FileExt};

use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_registry::WlRegistry, wl_shm::{self, WlShm}, wl_shm_pool::WlShmPool},
};
use wayland_protocols::ext::{
    foreign_toplevel_list::v1::client::{
        ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
        ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
    },
    image_capture_source::v1::client::{
        ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
        ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
    },
    image_copy_capture::v1::client::{
        ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
        ext_image_copy_capture_manager_v1::{ExtImageCopyCaptureManagerV1, Options},
        ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
    },
};

#[derive(Default)]
struct App {
    toplevels: Vec<ExtForeignToplevelHandleV1>,
    titles: Vec<String>,
    size: (u32, u32),
    shm_formats: Vec<u32>,
    constraints_done: bool,
    ready: bool,
    failed: Option<u32>,
}

impl Dispatch<WlRegistry, GlobalListContents> for App {
    fn event(_: &mut Self, _: &WlRegistry, _: wayland_client::protocol::wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ExtForeignToplevelListV1, ()> for App {
    fn event(state: &mut Self, _: &ExtForeignToplevelListV1, event: ext_foreign_toplevel_list_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            state.toplevels.push(toplevel);
        }
    }
    wayland_client::event_created_child!(App, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for App {
    fn event(state: &mut Self, _: &ExtForeignToplevelHandleV1, event: ext_foreign_toplevel_handle_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let ext_foreign_toplevel_handle_v1::Event::Title { title } = event {
            state.titles.push(title);
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for App {
    fn event(state: &mut Self, _: &ExtImageCopyCaptureSessionV1, event: ext_image_copy_capture_session_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use ext_image_copy_capture_session_v1::Event;
        match event {
            Event::BufferSize { width, height } => state.size = (width, height),
            Event::ShmFormat { format: WEnum::Value(format) } => state.shm_formats.push(format as u32),
            Event::Done => state.constraints_done = true,
            Event::Stopped => state.failed = Some(99),
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for App {
    fn event(state: &mut Self, _: &ExtImageCopyCaptureFrameV1, event: ext_image_copy_capture_frame_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            ext_image_copy_capture_frame_v1::Event::Ready => state.ready = true,
            ext_image_copy_capture_frame_v1::Event::Failed { reason } => {
                state.failed = Some(match reason {
                    WEnum::Value(reason) => reason as u32,
                    WEnum::Unknown(n) => n,
                })
            }
            _ => {}
        }
    }
}

delegate_noop!(App: ignore WlShm);
delegate_noop!(App: ignore WlShmPool);
delegate_noop!(App: ignore WlBuffer);
delegate_noop!(App: ignore WlOutput);
delegate_noop!(App: ignore ExtImageCaptureSourceV1);
delegate_noop!(App: ignore ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(App: ignore ExtForeignToplevelImageCaptureSourceManagerV1);
delegate_noop!(App: ignore ExtImageCopyCaptureManagerV1);

fn main() {
    let kind = std::env::args().nth(1).unwrap_or_else(|| "output".into());
    let conn = Connection::connect_to_env().expect("connect");
    let (globals, mut queue) = registry_queue_init::<App>(&conn).expect("registry");
    let qh = queue.handle();
    let mut app = App::default();

    let shm: WlShm = globals.bind(&qh, 1..=1, ()).expect("wl_shm");
    let manager: ExtImageCopyCaptureManagerV1 = globals.bind(&qh, 1..=1, ()).expect("ext_image_copy_capture_manager_v1");
    let source = if kind == "toplevel" {
        let list: ExtForeignToplevelListV1 = globals.bind(&qh, 1..=1, ()).expect("ext_foreign_toplevel_list_v1");
        let sources: ExtForeignToplevelImageCaptureSourceManagerV1 = globals.bind(&qh, 1..=1, ()).expect("toplevel source manager");
        queue.roundtrip(&mut app).unwrap();
        queue.roundtrip(&mut app).unwrap();
        let Some(toplevel) = app.toplevels.first().cloned() else { panic!("no toplevel announced") };
        let source = sources.create_source(&toplevel, &qh, ());
        drop(list);
        source
    } else {
        let sources: ExtOutputImageCaptureSourceManagerV1 = globals.bind(&qh, 1..=1, ()).expect("output source manager");
        let output: WlOutput = globals.bind(&qh, 1..=4, ()).expect("wl_output");
        sources.create_source(&output, &qh, ())
    };
    let session = manager.create_session(&source, Options::PaintCursors, &qh, ());
    while !app.constraints_done {
        queue.blocking_dispatch(&mut app).unwrap();
        if app.failed.is_some() {
            panic!("session stopped");
        }
    }
    let (w, h) = app.size;
    assert!(app.shm_formats.contains(&(wl_shm::Format::Argb8888 as u32)), "no Argb8888 offered");
    let stride = w * 4;
    let path = std::env::temp_dir().join(format!("mywm-capture-{}", std::process::id()));
    let file = File::options().read(true).write(true).create(true).truncate(true).open(&path).unwrap();
    file.set_len(u64::from(stride * h)).unwrap();
    let _ = std::fs::remove_file(&path);
    let pool = shm.create_pool(file.as_fd(), (stride * h) as i32, &qh, ());
    let buffer = pool.create_buffer(0, w as i32, h as i32, stride as i32, wl_shm::Format::Argb8888, &qh, ());

    let frame = session.create_frame(&qh, ());
    frame.attach_buffer(&buffer);
    frame.damage_buffer(0, 0, w as i32, h as i32);
    frame.capture();
    while !app.ready && app.failed.is_none() {
        queue.blocking_dispatch(&mut app).unwrap();
    }
    if let Some(reason) = app.failed {
        eprintln!("capture failed: {reason}");
        std::process::exit(2);
    }
    let mut pixel = [0u8; 4];
    file.read_exact_at(&mut pixel, u64::from((h / 2) * stride + (w / 2) * 4)).unwrap();
    // Argb8888 is little endian: B, G, R, A.
    println!("{w}x{h} center={:02X}{:02X}{:02X} alpha={:02X}", pixel[2], pixel[1], pixel[0], pixel[3]);
}
