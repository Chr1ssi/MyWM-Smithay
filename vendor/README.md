# Vendored dependencies

## smithay 0.7.0 (patched)

`vendor/smithay` is the unmodified crates.io release of smithay 0.7.0 plus the
changes in `smithay-async-flip.patch`:

- `DrmSurface::set_async_flip` / `DrmCompositor::set_async_flip`: following atomic page
  flips are submitted with `DRM_MODE_PAGE_FLIP_ASYNC`, i.e. they complete immediately
  instead of at the next vblank. This is what makes tearing (lowest latency for games that
  ask for it via `wp_tearing_control_v1`) possible; smithay 0.7 has no such switch.
- `#![allow(warnings)]` in `src/lib.rs` so the vendored code does not clutter our build.

The root `Cargo.toml` redirects `smithay` here with `[patch.crates-io]`.

To drop the patch once smithay supports asynchronous flips upstream, delete the
`[patch.crates-io]` section, `vendor/smithay` and this directory, and change
`set_async_flip` in `src/udev.rs` to the upstream API.

To check what differs from the release: unpack smithay 0.7.0 next to `vendor/smithay`
and run `diff -ru smithay-0.7.0 vendor/smithay`.
