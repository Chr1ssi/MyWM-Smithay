//! Usage: `mywm-test-client simple <seconds>` (a window that fills the size it is given with a color that
//! changes all the time; stands in for `weston-simple-shm`, which nixpkgs does not ship),
//! `mywm-test-client inhibit <seconds>` (a window that inhibits compositor shortcuts),
//! `mywm-test-client idle-inhibit <seconds>` (a window that keeps the session from going idle) or
//! `mywm-test-client urgent <seconds>` (two windows; the second asks to activate the first
//! without any user input behind the request).
use std::{os::fd::AsFd, time::{Duration, Instant}};

use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_keyboard::{self, WlKeyboard},
        wl_buffer::WlBuffer, wl_compositor::WlCompositor, wl_registry::WlRegistry, wl_seat::WlSeat, wl_shm::{self, WlShm},
        wl_shm_pool::WlShmPool, wl_surface::WlSurface,
    },
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, KeyboardInteractivity, ZwlrLayerSurfaceV1},
};
use wayland_protocols::{
    wp::idle_inhibit::zv1::client::{
        zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1, zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1,
    },
    wp::keyboard_shortcuts_inhibit::zv1::client::{
        zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1,
        zwp_keyboard_shortcuts_inhibitor_v1::{self, ZwpKeyboardShortcutsInhibitorV1},
    },
    xdg::{
        activation::v1::client::{
            xdg_activation_token_v1::{self, XdgActivationTokenV1},
            xdg_activation_v1::XdgActivationV1,
        },
        shell::client::{
            xdg_popup::{self, XdgPopup},
            xdg_positioner::{Anchor, ConstraintAdjustment, Gravity, XdgPositioner},
            xdg_surface::{self, XdgSurface},
            xdg_toplevel::XdgToplevel,
            xdg_wm_base::{self, XdgWmBase},
        },
    },
};

#[derive(Default)]
struct App {
    keyboard_entered: bool,
    configured: Vec<bool>,
    inhibitor_active: Option<bool>,
    token: Option<String>,
    popup: Option<(i32, i32, i32, i32)>,
    /// The size the compositor last gave the toplevel (0x0: the client chooses).
    toplevel_size: (i32, i32),
}

impl Dispatch<WlRegistry, GlobalListContents> for App {
    fn event(_: &mut Self, _: &WlRegistry, _: wayland_client::protocol::wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<XdgWmBase, ()> for App {
    fn event(_: &mut Self, base: &XdgWmBase, event: xdg_wm_base::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            base.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, usize> for App {
    fn event(state: &mut Self, surface: &XdgSurface, event: xdg_surface::Event, index: &usize, _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
            state.configured[*index] = true;
        }
    }
}

impl Dispatch<XdgToplevel, ()> for App {
    fn event(state: &mut Self, _: &XdgToplevel, event: wayland_protocols::xdg::shell::client::xdg_toplevel::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wayland_protocols::xdg::shell::client::xdg_toplevel::Event::Configure { width, height, .. } = event {
            state.toplevel_size = (width, height);
        }
    }
}

impl Dispatch<ZwpKeyboardShortcutsInhibitorV1, ()> for App {
    fn event(state: &mut Self, _: &ZwpKeyboardShortcutsInhibitorV1, event: zwp_keyboard_shortcuts_inhibitor_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            zwp_keyboard_shortcuts_inhibitor_v1::Event::Active => state.inhibitor_active = Some(true),
            zwp_keyboard_shortcuts_inhibitor_v1::Event::Inactive => state.inhibitor_active = Some(false),
            _ => {}
        }
    }
}

impl Dispatch<XdgActivationTokenV1, ()> for App {
    fn event(state: &mut Self, _: &XdgActivationTokenV1, event: xdg_activation_token_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_activation_token_v1::Event::Done { token } = event {
            state.token = Some(token);
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for App {
    fn event(state: &mut Self, surface: &ZwlrLayerSurfaceV1, event: zwlr_layer_surface_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
            surface.ack_configure(serial);
            state.configured.iter_mut().for_each(|c| *c = true);
        }
    }
}

impl Dispatch<WlKeyboard, ()> for App {
    fn event(state: &mut Self, _: &WlKeyboard, event: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_keyboard::Event::Enter { .. } = event {
            state.keyboard_entered = true;
        }
    }
}

delegate_noop!(App: ignore ZwlrLayerShellV1);
delegate_noop!(App: ignore WlCompositor);
delegate_noop!(App: ignore WlShm);
delegate_noop!(App: ignore WlShmPool);
delegate_noop!(App: ignore WlBuffer);
delegate_noop!(App: ignore WlSurface);
delegate_noop!(App: ignore WlSeat);
delegate_noop!(App: ignore ZwpKeyboardShortcutsInhibitManagerV1);
delegate_noop!(App: ignore ZwpIdleInhibitManagerV1);
delegate_noop!(App: ignore ZwpIdleInhibitorV1);
delegate_noop!(App: ignore XdgActivationV1);
delegate_noop!(App: ignore XdgPositioner);

impl Dispatch<XdgPopup, ()> for App {
    fn event(state: &mut Self, _: &XdgPopup, event: xdg_popup::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_popup::Event::Configure { x, y, width, height } = event {
            state.popup = Some((x, y, width, height));
        }
    }
}

struct Window {
    surface: WlSurface,
    _xdg: XdgSurface,
    _toplevel: XdgToplevel,
}

/// A toplevel of whatever size the compositor gives it, redrawn in a new color every 50 ms.
fn simple_window(app: &mut App, queue: &mut wayland_client::EventQueue<App>, compositor: &WlCompositor, shm: &WlShm, wm_base: &XdgWmBase, seconds: u64) {
    let qh = queue.handle();
    app.configured = vec![false];
    let surface = compositor.create_surface(&qh, ());
    let xdg = wm_base.get_xdg_surface(&surface, &qh, 0);
    let toplevel = xdg.get_toplevel(&qh, ());
    toplevel.set_app_id("mywm.test.simple".into());
    surface.commit();
    while !app.configured[0] {
        queue.blocking_dispatch(app).unwrap();
    }
    let file = std::fs::File::options().read(true).write(true).create(true).truncate(true).open(std::env::temp_dir().join(format!("mywm-tc-{}", std::process::id()))).unwrap();
    let end = Instant::now() + Duration::from_secs(seconds);
    let mut frame = 0u32;
    while Instant::now() < end {
        // Leave with the compositor: every error here means the connection is gone.
        if queue.dispatch_pending(app).is_err() {
            return;
        }
        let (w, h) = if app.toplevel_size.0 > 0 && app.toplevel_size.1 > 0 { app.toplevel_size } else { (256, 256) };
        let length = (w * h * 4) as usize;
        file.set_len(length as u64).unwrap();
        // A gradient from left to right whose phase moves, so the picture has many colors and changes every frame.
        let phase = frame.wrapping_mul(7) as usize;
        let row: Vec<u8> = (0..w as usize).flat_map(|x| [((x + phase) % 256) as u8, 0x40, 0xC0 - ((x * 128 / w as usize) as u8), 0xFF]).collect();
        let pixels: Vec<u8> = row.iter().copied().cycle().take(length).collect();
        std::os::unix::fs::FileExt::write_all_at(&file, &pixels, 0).unwrap();
        let pool = shm.create_pool(file.as_fd(), length as i32, &qh, ());
        let buffer = pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Xrgb8888, &qh, ());
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, w, h);
        surface.commit();
        if queue.flush().is_err() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
        if queue.roundtrip(app).is_err() {
            return;
        }
        buffer.destroy();
        pool.destroy();
        frame = frame.wrapping_add(1);
    }
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let seconds: u64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let conn = Connection::connect_to_env().expect("connect");
    let (globals, mut queue) = registry_queue_init::<App>(&conn).expect("registry");
    let qh = queue.handle();
    let mut app = App::default();

    let compositor: WlCompositor = globals.bind(&qh, 1..=4, ()).unwrap();
    let shm: WlShm = globals.bind(&qh, 1..=1, ()).unwrap();
    let wm_base: XdgWmBase = globals.bind(&qh, 1..=1, ()).unwrap();

    if mode == "simple" {
        simple_window(&mut app, &mut queue, &compositor, &shm, &wm_base, seconds);
        return;
    }

    if mode == "layer" || mode == "fullscreen" {
        // `layer <top|overlay> <w> <h> <RRGGBB> <seconds>` or `fullscreen <RRGGBB> <seconds>`: one solid surface.
        let args: Vec<String> = std::env::args().collect();
        let (layer_name, w, h, color, seconds) = if mode == "layer" {
            (args[2].clone(), args[3].parse::<i32>().unwrap(), args[4].parse::<i32>().unwrap(), args[5].clone(), args[6].parse::<u64>().unwrap())
        } else {
            (String::new(), 1280, 800, args[2].clone(), args[3].parse::<u64>().unwrap())
        };
        let rgb = u32::from_str_radix(&color, 16).unwrap();
        let file = std::fs::File::options().read(true).write(true).create(true).truncate(true).open(std::env::temp_dir().join(format!("mywm-tc-{}", std::process::id()))).unwrap();
        file.set_len((w * h * 4) as u64).unwrap();
        let pixel = (0xff00_0000u32 | rgb).to_le_bytes();
        let pixels: Vec<u8> = (0..w * h).flat_map(|_| pixel).collect();
        std::os::unix::fs::FileExt::write_all_at(&file, &pixels, 0).unwrap();
        let pool = shm.create_pool(file.as_fd(), w * h * 4, &qh, ());
        let buffer = pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Xrgb8888, &qh, ());
        let surface = compositor.create_surface(&qh, ());
        app.configured = vec![false];
        let _keep: (Option<ZwlrLayerSurfaceV1>, Option<(XdgSurface, XdgToplevel)>) = if mode == "layer" {
            let shell: ZwlrLayerShellV1 = globals.bind(&qh, 1..=4, ()).expect("layer shell");
            let layer = if layer_name == "overlay" { Layer::Overlay } else { Layer::Top };
            let layer_surface = shell.get_layer_surface(&surface, None, layer, "test".into(), &qh, ());
            layer_surface.set_size(w as u32, h as u32);
            let exclusive = args.get(7).is_some_and(|a| a == "exclusive");
            if exclusive {
                layer_surface.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
            }
            surface.commit();
            while !app.configured[0] {
                queue.blocking_dispatch(&mut app).unwrap();
            }
            (Some(layer_surface), None)
        } else {
            let xdg = wm_base.get_xdg_surface(&surface, &qh, 0);
            let toplevel = xdg.get_toplevel(&qh, ());
            toplevel.set_app_id("mywm.test.fullscreen".into());
            toplevel.set_fullscreen(None);
            surface.commit();
            while !app.configured[0] {
                queue.blocking_dispatch(&mut app).unwrap();
            }
            (None, Some((xdg, toplevel)))
        };
        surface.attach(Some(&buffer), 0, 0);
        surface.commit();
        queue.roundtrip(&mut app).unwrap();
        println!("shown");
        let seat: WlSeat = globals.bind(&qh, 1..=1, ()).unwrap();
        let _keyboard = seat.get_keyboard(&qh, ());
        let end = Instant::now() + Duration::from_secs(seconds);
        let mut reported = false;
        while Instant::now() < end {
            let _ = queue.roundtrip(&mut app);
            if app.keyboard_entered && !reported {
                println!("keyboard focus");
                reported = true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        return;
    }

    let count = if mode == "urgent" { 2 } else { 1 };
    app.configured = vec![false; count];
    let (w, h) = (200i32, 150i32);
    let file = std::fs::File::options().read(true).write(true).create(true).truncate(true).open(std::env::temp_dir().join(format!("mywm-tc-{}", std::process::id()))).unwrap();
    file.set_len((w * h * 4) as u64).unwrap();
    let pixels = vec![0x80u8; (w * h * 4) as usize];
    std::os::unix::fs::FileExt::write_all_at(&file, &pixels, 0).unwrap();
    let pool = shm.create_pool(file.as_fd(), w * h * 4, &qh, ());
    let buffer = pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Xrgb8888, &qh, ());

    let mut windows = Vec::new();
    for index in 0..count {
        let surface = compositor.create_surface(&qh, ());
        let xdg = wm_base.get_xdg_surface(&surface, &qh, index);
        let toplevel = xdg.get_toplevel(&qh, ());
        toplevel.set_app_id(format!("mywm.test.{index}"));
        surface.commit();
        windows.push(Window { surface, _xdg: xdg, _toplevel: toplevel });
        // One window at a time so the second is the newest (and focused).
        while !app.configured[index] {
            queue.blocking_dispatch(&mut app).unwrap();
        }
        windows[index].surface.attach(Some(&buffer), 0, 0);
        windows[index].surface.commit();
        queue.roundtrip(&mut app).unwrap();
        std::thread::sleep(Duration::from_millis(400));
    }

    match mode.as_str() {
        "inhibit" => {
            let manager: ZwpKeyboardShortcutsInhibitManagerV1 = globals.bind(&qh, 1..=1, ()).expect("shortcuts inhibit manager");
            let seat: WlSeat = globals.bind(&qh, 1..=1, ()).unwrap();
            let _inhibitor = manager.inhibit_shortcuts(&windows[0].surface, &seat, &qh, ());
            queue.roundtrip(&mut app).unwrap();
            println!("inhibitor {:?}", app.inhibitor_active);
        }
        "idle-inhibit" => {
            // Like Chromium: replace the inhibitor by creating the new one before destroying the old one.
            // The surviving inhibitor is never destroyed; it goes away with the connection on exit.
            let manager: ZwpIdleInhibitManagerV1 = globals.bind(&qh, 1..=1, ()).expect("idle inhibit manager");
            let first = manager.create_inhibitor(&windows[0].surface, &qh, ());
            let _second = manager.create_inhibitor(&windows[0].surface, &qh, ());
            first.destroy();
            queue.roundtrip(&mut app).unwrap();
            println!("inhibiting");
        }
        "urgent" => {
            let activation: XdgActivationV1 = globals.bind(&qh, 1..=1, ()).expect("xdg_activation_v1");
            let token = activation.get_activation_token(&qh, ());
            token.commit();
            while app.token.is_none() {
                queue.blocking_dispatch(&mut app).unwrap();
            }
            // The first window is not focused (the second is): ask for attention.
            activation.activate(app.token.clone().unwrap(), &windows[0].surface);
            queue.roundtrip(&mut app).unwrap();
            println!("activation requested");
        }
        "popup" => {
            // A popup anchored at the bottom right corner of the window, pointing down and right:
            // it must be configured, and a compositor may flip it to stay on screen.
            let positioner = wm_base.create_positioner(&qh, ());
            positioner.set_size(100, 80);
            positioner.set_anchor_rect(0, 0, w, h);
            positioner.set_anchor(Anchor::BottomRight);
            positioner.set_gravity(Gravity::BottomRight);
            positioner.set_constraint_adjustment(ConstraintAdjustment::all());
            let surface = compositor.create_surface(&qh, ());
            let popup_xdg = wm_base.get_xdg_surface(&surface, &qh, 0);
            let popup = popup_xdg.get_popup(Some(&windows[0]._xdg), &positioner, &qh, ());
            surface.commit();
            let deadline = Instant::now() + Duration::from_secs(3);
            while app.popup.is_none() && Instant::now() < deadline {
                queue.roundtrip(&mut app).unwrap();
                std::thread::sleep(Duration::from_millis(50));
            }
            match app.popup {
                Some((x, y, w, h)) => println!("popup configured {x} {y} {w} {h}"),
                None => println!("popup never configured"),
            }
            let _keep = (popup, popup_xdg, surface);
            std::thread::sleep(Duration::from_secs(seconds));
            return;
        }
        other => panic!("unknown mode {other}"),
    }
    let end = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < end {
        if queue.dispatch_pending(&mut app).is_err() {
            break;
        }
        let _ = queue.flush();
        if let Some(guard) = queue.prepare_read() {
            // Wait a little for events without blocking forever.
            let mut pfd = libc::pollfd { fd: std::os::fd::AsRawFd::as_raw_fd(&guard.connection_fd()), events: libc::POLLIN, revents: 0 };
            // SAFETY: a valid pollfd for a single descriptor.
            let ready = unsafe { libc::poll(&mut pfd, 1, 100) };
            if ready > 0 {
                let _ = guard.read();
            }
        }
    }
    drop(buffer);
}
