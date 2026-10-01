# Vendored dependencies

## smithay 0.7.0 (patched)

`vendor/smithay` is the unmodified crates.io release of smithay 0.7.0 plus the
changes in `smithay.patch`:

- `DrmSurface::set_async_flip` / `DrmCompositor::set_async_flip`: following atomic page
  flips are submitted with `DRM_MODE_PAGE_FLIP_ASYNC`, i.e. they complete immediately
  instead of at the next vblank. This is what makes tearing (lowest latency for games that
  ask for it via `wp_tearing_control_v1`) possible; smithay 0.7 has no such switch.
- Layer shell: a surface that commits a zero size without anchoring both edges (Quickshell does
  this briefly when it recreates a panel) is no longer a protocol error that disconnects the
  client, which used to kill the whole bar. `LayerMap::arrange` shows such a surface at 1px until
  the client sends a proper size.
- `X11Surface::set_x11_input_focus`: the X11 half of `KeyboardTarget::enter/leave`. Our keyboard
  focus is a plain `wl_surface`, so without it Xwayland never gets an X11 input focus and Wine/Proton
  games (which wait for `WM_TAKE_FOCUS`) receive no keys.
- `X11Wm::new_selection` flushes the X connection. The owner change otherwise stays buffered until
  another X event arrives, so a freshly copied Wayland selection was invisible to X11 clients.
- `#![allow(warnings)]` in `src/lib.rs` so the vendored code does not clutter our build.

The root `Cargo.toml` redirects `smithay` here with `[patch.crates-io]`.

To drop a change once smithay supports it upstream, adjust `smithay.patch`; to drop all of them, delete the
`[patch.crates-io]` section, `vendor/smithay` and this directory, and change
`set_async_flip` in `src/udev.rs` to the upstream API.

To check what differs from the release: unpack smithay 0.7.0 next to `vendor/smithay`
and run `diff -ru smithay-0.7.0 vendor/smithay`.
