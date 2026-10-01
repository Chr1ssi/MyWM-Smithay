# MyWM-Smithay

Smithay-based Wayland compositor: lightweight, with a small set of selected visual
effects. Priorities, in this order of weight: **stability**, **ease of use**, **low
latency** (especially for gaming). Primary target is **NixOS**.

## Principles

- **No hacks.** No sketchy workarounds, no timing-based band-aids, no special-casing
  of single applications. Find the root cause and fix it properly. If a workaround is
  truly unavoidable (e.g. a compositor-side bug in a client or in smithay), isolate it,
  document why it is needed and when it can go, and tell the user.
- **Think before changing.** Understand the surrounding code and the relevant protocol
  or kernel behavior first. No speculative or "try and see" edits, no drive-by refactors.
- **Optimize deliberately.** Optimize wherever it is sensible, especially on the render,
  input and frame-pacing paths (no needless allocations, copies, redraws or blocking
  calls). Back non-trivial claims with a measurement (`RUST_LOG=info,perf=debug`) and
  never trade stability or correctness for a speculative gain.
- **Latency first on the gaming path.** Direct scanout, tearing, VRR, explicit sync and
  per-output redraws must not regress. Effects must never cost frames in fullscreen or
  scanout situations.
- **Stability over features.** A crash or hang takes the whole session down. Avoid
  `unwrap`/`expect` on anything that depends on clients, hardware or the environment;
  a misbehaving client must never be able to kill the compositor.
- **Ease of use.** Sensible defaults, a small config surface, clear error messages.
  Keep the config schema compatible with the River-MyWM and the bar protocol (`mywm-ipc`)
  compatible with `mywm-shell` unless the user decides otherwise.
- **Effects stay selective.** Rounded corners, transparency, shadows, animations and
  blur exist already (`docs/effects.md`). New effects need a clear benefit, must be
  switchable, and must be cheap.

## Logging

- Logging added for debugging is temporary. **Remove it once the problem is solved**,
  before committing, unless it has lasting diagnostic value (errors, warnings, rare
  state changes, the existing `perf` target).
- No logging in per-frame or per-input-event hot paths in committed code.
- Never leave debug output, `dbg!`, `println!`, or temporary env switches behind.

## NixOS compatibility

- Everything must work on NixOS: no hardcoded FHS paths (`/usr/...`, `/bin/...`); resolve
  tools via `PATH` or from the Nix store, use XDG directories for user data.
- Keep `flake.nix` and `nix/` (`package.nix`, `module.nix`, `settings.nix`) in sync with
  code changes: new dependencies, binaries, runtime tools, environment variables, session
  scripts (`scripts/`) and options belong in the Nix expressions as well.
- Changes that touch the build or packaging must be verified with `nix build`, not only
  with `cargo build`. Keep `flake.lock` and `Cargo.lock` consistent.
- Do not rely on `/etc`-style imperative setup; configuration is declarative where possible.

## Code

- Match the surrounding style. Code and comments are in English, docs (`README.md`,
  `docs/`) are in German. Comments explain *why*, not *what*.
- Smithay is vendored in `vendor/smithay` with a small patch set (`vendor/README.md`).
  Keep that patch set minimal, prefer upstream APIs, and update `vendor/README.md`
  whenever the patches change.
- `crates/mywm-layout`, `mywm-config` and `mywm-ipc` stay free of compositor
  dependencies and keep their unit tests.
- Update `README.md` / `docs/` when behavior, config options or protocols change, and
  `crates/mywm-config/example.toml` when the schema changes.

## Testing

- Run before every commit, inside `nix develop`: `cargo clippy --workspace --all-targets` (no new
  warnings), `cargo test --workspace`, and `nix build` (which also runs the unit tests).
- Run the matching nested smoke tests from `tests/` for what you touched
  (`PYTHONPATH=tests python3 tests/<name>_smoke.py`), and `globals_smoke.py` when
  protocols change. Add or extend a test for new behavior where practical.
- Anything that needs real hardware (DRM, libinput, VRR, tearing, scanout, gamma) cannot
  be verified nested. Say so explicitly instead of claiming it works, and point to
  `docs/hardware-test.md`. Such changes go on their own branch (see Git workflow).

## Git workflow

- Changes that you could fully test yourself (build, `nix build`, tests, nested smoke
  tests) are committed and pushed to `master` without asking for additional confirmation,
  so the user can rebuild and test them immediately.
- Changes that can only be verified on real hardware go on their own branch
  (descriptive name, e.g. `hw/<topic>`) and are pushed there, never to `master`. Tell the
  user what needs testing; they merge after the hardware test.
- Do not push changes that have not passed the relevant checks; report blockers instead.
- Small, focused commits with an imperative, descriptive subject line (see `git log`).
- When `mywm-shell` or this repo change in a way that
  affects dependents, update and test the dependent `flake.lock` files in dependency
  order (`mywm-shell` → this repo → the NixOS configuration).
- Never force-push, rewrite published history, or commit build output (`target/`,
  `result`).
