//! `wlr-gamma-control`: lets night-light tools (gammastep, wlsunset) set a color ramp per
//! output. Only offered on hardware; the ramp goes to the CRTC's gamma table.
use std::{io::Read, os::fd::OwnedFd};

use smithay::{
    output::Output,
    reexports::{
        wayland_protocols_wlr::gamma_control::v1::server::{
            zwlr_gamma_control_manager_v1::{self, ZwlrGammaControlManagerV1},
            zwlr_gamma_control_v1::{self, ZwlrGammaControlV1},
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
            backend::{ClientId, GlobalId},
        },
    },
};

use crate::State;

/// Red, green and blue table of a ramp.
pub type Ramp = (Vec<u16>, Vec<u16>, Vec<u16>);

/// The client holding an output's ramp.
#[derive(Default)]
pub struct GammaOwners {
    owners: Vec<(Output, ZwlrGammaControlV1)>,
}

pub struct GammaData {
    output: Output,
    size: u32,
}

impl State {
    pub fn create_gamma_global(display: &DisplayHandle) -> GlobalId {
        display.create_global::<State, ZwlrGammaControlManagerV1, ()>(1, ())
    }
}

impl GlobalDispatch<ZwlrGammaControlManagerV1, (), State> for State {
    fn bind(_: &mut State, _: &DisplayHandle, _: &Client, resource: New<ZwlrGammaControlManagerV1>, _: &(), init: &mut DataInit<'_, State>) {
        init.init(resource, ());
    }
}

impl Dispatch<ZwlrGammaControlManagerV1, (), State> for State {
    fn request(
        state: &mut State,
        _: &Client,
        _: &ZwlrGammaControlManagerV1,
        request: zwlr_gamma_control_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, State>,
    ) {
        if let zwlr_gamma_control_manager_v1::Request::GetGammaControl { id, output } = request {
            let output = Output::from_resource(&output);
            let size = output.as_ref().and_then(|o| state.udev.as_ref()?.gamma_size(o)).unwrap_or(0);
            let taken = output.as_ref().is_some_and(|o| state.gamma.owners.iter().any(|(owner, c)| owner == o && c.is_alive()));
            let Some(output) = output.filter(|_| size > 0 && !taken) else {
                // No such output, no gamma table, or another client already holds it.
                let dead = Output::new(
                    String::new(),
                    smithay::output::PhysicalProperties {
                        size: (0, 0).into(),
                        subpixel: smithay::output::Subpixel::Unknown,
                        make: String::new(),
                        model: String::new(),
                    },
                );
                init.init(id, GammaData { output: dead, size: 0 }).failed();
                return;
            };
            let control = init.init(id, GammaData { output: output.clone(), size });
            control.gamma_size(size);
            state.gamma.owners.push((output, control));
        }
    }
}

impl Dispatch<ZwlrGammaControlV1, GammaData, State> for State {
    fn request(
        state: &mut State,
        _: &Client,
        control: &ZwlrGammaControlV1,
        request: zwlr_gamma_control_v1::Request,
        data: &GammaData,
        _: &DisplayHandle,
        _: &mut DataInit<'_, State>,
    ) {
        if let zwlr_gamma_control_v1::Request::SetGamma { fd } = request {
            if data.size == 0 {
                return;
            }
            match read_ramp(fd, data.size as usize) {
                Some(ramp) => {
                    if !state.udev.as_mut().is_some_and(|u| u.apply_gamma(&data.output, Some(ramp))) {
                        control.failed();
                    }
                }
                None => control.failed(),
            }
        }
    }

    fn destroyed(state: &mut State, _: ClientId, control: &ZwlrGammaControlV1, data: &GammaData) {
        let was_owner = state.gamma.owners.iter().any(|(_, c)| c == control);
        state.gamma.owners.retain(|(_, c)| c != control);
        // The client is gone: the output goes back to its normal colors.
        if was_owner && let Some(udev) = &mut state.udev {
            udev.apply_gamma(&data.output, None);
        }
    }
}

/// The three tables (`size` entries each, native endian) a client wrote into `fd`.
fn read_ramp(fd: OwnedFd, size: usize) -> Option<Ramp> {
    let mut file = std::fs::File::from(fd);
    let mut bytes = vec![0u8; size * 3 * 2];
    file.read_exact(&mut bytes).ok()?;
    // Exactly three tables: anything more is a broken client.
    let mut extra = [0u8; 1];
    if file.read(&mut extra).ok()? != 0 {
        return None;
    }
    let values: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_ne_bytes([c[0], c[1]])).collect();
    Some((values[..size].to_vec(), values[size..2 * size].to_vec(), values[2 * size..].to_vec()))
}

/// A straight ramp (no color change) of `size` entries.
pub fn identity_ramp(size: usize) -> Ramp {
    let table: Vec<u16> = (0..size).map(|i| ((i as u64 * 65535) / (size.max(2) as u64 - 1)) as u16).collect();
    (table.clone(), table.clone(), table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fd_with(bytes: &[u8]) -> OwnedFd {
        let mut file = tempfile_in_memory();
        file.write_all(bytes).unwrap();
        std::io::Seek::rewind(&mut file).unwrap();
        file.into()
    }

    fn tempfile_in_memory() -> std::fs::File {
        let fd = unsafe { libc::memfd_create(c"ramp".as_ptr(), 0) };
        assert!(fd >= 0);
        unsafe { std::os::fd::FromRawFd::from_raw_fd(fd) }
    }

    #[test]
    fn a_ramp_needs_exactly_three_tables() {
        let mut bytes = Vec::new();
        for v in [1u16, 2, 3, 4, 5, 6] {
            bytes.extend_from_slice(&v.to_ne_bytes());
        }
        assert_eq!(read_ramp(fd_with(&bytes), 2), Some((vec![1, 2], vec![3, 4], vec![5, 6])));
        assert_eq!(read_ramp(fd_with(&bytes[..10]), 2), None);
        let mut long = bytes.clone();
        long.extend_from_slice(&[0, 0]);
        assert_eq!(read_ramp(fd_with(&long), 2), None);
    }

    #[test]
    fn the_identity_ramp_spans_the_full_range() {
        let (r, _, _) = identity_ramp(256);
        assert_eq!((r[0], r[255]), (0, 65535));
        assert!(r.windows(2).all(|w| w[0] < w[1]));
    }
}
