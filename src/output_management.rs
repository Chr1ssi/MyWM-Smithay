//! `wlr-output-management`: lets kanshi, wlr-randr and friends change mode, position, scale
//! and transform of the outputs at runtime.
//!
//! Heads (one per output) are rebuilt whenever an output appears, vanishes or changes; clients
//! that cache state handle `finished` heads. Switching an output off is not supported.
use std::sync::{Arc, Mutex};

use smithay::{
    output::{Mode, Output, Scale},
    reexports::{
        wayland_protocols_wlr::output_management::v1::server::{
            zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
            zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
            zwlr_output_head_v1::ZwlrOutputHeadV1,
            zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
            zwlr_output_mode_v1::ZwlrOutputModeV1,
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
            backend::GlobalId,
            protocol::wl_output::Transform as WlTransform,
        },
    },
    utils::{Logical, Point, Transform},
};

use crate::State;

struct Head {
    resource: ZwlrOutputHeadV1,
    modes: Vec<ZwlrOutputModeV1>,
}

struct Manager {
    resource: ZwlrOutputManagerV1,
    heads: Vec<Head>,
}

#[derive(Default)]
pub struct OutputManagement {
    managers: Vec<Manager>,
    serial: u32,
}

/// What a client asked for one output.
struct HeadChange {
    output: Output,
    enabled: bool,
    mode: Option<Mode>,
    position: Option<Point<i32, Logical>>,
    transform: Option<Transform>,
    scale: Option<f64>,
}

#[derive(Default)]
pub struct ConfigState {
    serial: u32,
    used: bool,
    heads: Vec<HeadChange>,
}

type Shared = Arc<Mutex<ConfigState>>;

pub struct ConfigHeadData {
    state: Shared,
    output: Output,
}

fn transform_from(transform: WlTransform) -> Option<Transform> {
    Some(match transform {
        WlTransform::Normal => Transform::Normal,
        WlTransform::_90 => Transform::_90,
        WlTransform::_180 => Transform::_180,
        WlTransform::_270 => Transform::_270,
        WlTransform::Flipped => Transform::Flipped,
        WlTransform::Flipped90 => Transform::Flipped90,
        WlTransform::Flipped180 => Transform::Flipped180,
        WlTransform::Flipped270 => Transform::Flipped270,
        _ => return None,
    })
}

impl State {
    pub fn create_output_management_global(display: &DisplayHandle) -> GlobalId {
        display.create_global::<State, ZwlrOutputManagerV1, ()>(4, ())
    }

    /// Outputs changed: send every client a fresh set of heads.
    pub fn output_management_changed(&mut self) {
        let outputs: Vec<Output> = self.outputs.iter().map(|e| e.output.clone()).collect();
        let dh = self.display_handle.clone();
        self.output_management.serial = self.output_management.serial.wrapping_add(1);
        let serial = self.output_management.serial;
        for manager in &mut self.output_management.managers {
            for head in manager.heads.drain(..) {
                for mode in &head.modes {
                    mode.finished();
                }
                head.resource.finished();
            }
            if let Some(client) = manager.resource.client() {
                manager.heads = outputs.iter().filter_map(|o| send_head(&dh, &client, &manager.resource, o)).collect();
            }
            manager.resource.done(serial);
        }
    }

    /// Change the mode, transform, scale and position of outputs as requested; all or nothing
    /// as far as validation goes (`test_only` stops after it).
    fn apply_output_config(&mut self, config: &ConfigState, test_only: bool) -> bool {
        for change in &config.heads {
            if !change.enabled || !self.outputs.iter().any(|e| e.output == change.output) {
                // Switching outputs off (or configuring ones that vanished) is not supported.
                return false;
            }
            if let Some(mode) = change.mode
                && !change.output.modes().contains(&mode)
            {
                return false;
            }
            if change.scale.is_some_and(|s| !(s.is_finite() && s > 0.0)) {
                return false;
            }
        }
        if test_only {
            return true;
        }
        for change in &config.heads {
            let output = &change.output;
            if let Some(mode) = change.mode
                && output.current_mode() != Some(mode)
                && !self.set_output_mode(output, mode)
            {
                return false;
            }
            output.change_current_state(
                change.mode,
                change.transform,
                change.scale.map(Scale::Fractional),
                change.position,
            );
            if let Some(position) = change.position {
                self.space.map_output(output, position);
            }
            smithay::desktop::layer_map_for_output(output).arrange();
        }
        self.refresh();
        for change in &config.heads {
            self.queue_redraw_output(&change.output);
        }
        true
    }
}

fn send_head(dh: &DisplayHandle, client: &Client, manager: &ZwlrOutputManagerV1, output: &Output) -> Option<Head> {
    let version = manager.version();
    let head = client.create_resource::<ZwlrOutputHeadV1, Output, State>(dh, version, output.clone()).ok()?;
    manager.head(&head);
    head.name(output.name());
    let props = output.physical_properties();
    head.description(format!("{} ({})", props.model, output.name()));
    if props.size.w > 0 && props.size.h > 0 {
        head.physical_size(props.size.w, props.size.h);
    }
    if version >= 2 {
        head.make(props.make.clone());
        head.model(props.model.clone());
    }
    let current = output.current_mode();
    let preferred = output.preferred_mode();
    let mut modes = Vec::new();
    let mut current_resource = None;
    for mode in output.modes() {
        let Ok(resource) = client.create_resource::<ZwlrOutputModeV1, Mode, State>(dh, version.min(3), mode) else { continue };
        head.mode(&resource);
        resource.size(mode.size.w, mode.size.h);
        resource.refresh(mode.refresh);
        if preferred == Some(mode) {
            resource.preferred();
        }
        if current == Some(mode) {
            current_resource = Some(resource.clone());
        }
        modes.push(resource);
    }
    head.enabled(1);
    if let Some(resource) = &current_resource {
        head.current_mode(resource);
    }
    let location = output.current_location();
    head.position(location.x, location.y);
    head.transform(output.current_transform().into());
    head.scale(output.current_scale().fractional_scale());
    Some(Head { resource: head, modes })
}

impl GlobalDispatch<ZwlrOutputManagerV1, (), State> for State {
    fn bind(
        state: &mut State,
        dh: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrOutputManagerV1>,
        _: &(),
        data_init: &mut DataInit<'_, State>,
    ) {
        let manager = data_init.init(resource, ());
        let heads = state
            .outputs
            .iter()
            .filter_map(|e| send_head(dh, client, &manager, &e.output))
            .collect();
        manager.done(state.output_management.serial);
        state.output_management.managers.push(Manager { resource: manager, heads });
    }
}

impl Dispatch<ZwlrOutputManagerV1, (), State> for State {
    fn request(
        state: &mut State,
        _: &Client,
        manager: &ZwlrOutputManagerV1,
        request: zwlr_output_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_output_manager_v1::Request::CreateConfiguration { id, serial } => {
                let _ = manager;
                data_init.init(id, Arc::new(Mutex::new(ConfigState { serial, ..Default::default() })));
            }
            zwlr_output_manager_v1::Request::Stop => {
                manager.finished();
                state.output_management.managers.retain(|m| m.resource != *manager);
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut State, _: smithay::reexports::wayland_server::backend::ClientId, manager: &ZwlrOutputManagerV1, _: &()) {
        state.output_management.managers.retain(|m| m.resource != *manager);
    }
}

impl Dispatch<ZwlrOutputHeadV1, Output, State> for State {
    fn request(
        _: &mut State,
        _: &Client,
        _: &ZwlrOutputHeadV1,
        _: smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_head_v1::Request,
        _: &Output,
        _: &DisplayHandle,
        _: &mut DataInit<'_, State>,
    ) {
    }
}

impl Dispatch<ZwlrOutputModeV1, Mode, State> for State {
    fn request(
        _: &mut State,
        _: &Client,
        _: &ZwlrOutputModeV1,
        _: smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_mode_v1::Request,
        _: &Mode,
        _: &DisplayHandle,
        _: &mut DataInit<'_, State>,
    ) {
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, Shared, State> for State {
    fn request(
        state: &mut State,
        _: &Client,
        resource: &ZwlrOutputConfigurationV1,
        request: zwlr_output_configuration_v1::Request,
        data: &Shared,
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        use zwlr_output_configuration_v1::Request;
        match request {
            Request::EnableHead { id, head } => {
                let Some(output) = head.data::<Output>().cloned() else { return };
                let mut config = data.lock().unwrap();
                if config.heads.iter().any(|h| h.output == output) {
                    resource.post_error(zwlr_output_configuration_v1::Error::AlreadyConfiguredHead, "head configured twice");
                    return;
                }
                config.heads.push(HeadChange { output: output.clone(), enabled: true, mode: None, position: None, transform: None, scale: None });
                drop(config);
                data_init.init(id, ConfigHeadData { state: data.clone(), output });
            }
            Request::DisableHead { head } => {
                let Some(output) = head.data::<Output>().cloned() else { return };
                let mut config = data.lock().unwrap();
                if config.heads.iter().any(|h| h.output == output) {
                    resource.post_error(zwlr_output_configuration_v1::Error::AlreadyConfiguredHead, "head configured twice");
                    return;
                }
                config.heads.push(HeadChange { output, enabled: false, mode: None, position: None, transform: None, scale: None });
            }
            Request::Apply | Request::Test => {
                let test_only = matches!(request, Request::Test);
                let mut config = data.lock().unwrap();
                if config.used {
                    resource.post_error(zwlr_output_configuration_v1::Error::AlreadyUsed, "configuration applied twice");
                    return;
                }
                config.used = true;
                if config.serial != state.output_management.serial {
                    resource.cancelled();
                    return;
                }
                let config = std::mem::take(&mut *config);
                if state.apply_output_config(&config, test_only) {
                    resource.succeeded();
                    if !test_only {
                        state.output_management_changed();
                    }
                } else {
                    resource.failed();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, ConfigHeadData, State> for State {
    fn request(
        _: &mut State,
        _: &Client,
        resource: &ZwlrOutputConfigurationHeadV1,
        request: zwlr_output_configuration_head_v1::Request,
        data: &ConfigHeadData,
        _: &DisplayHandle,
        _: &mut DataInit<'_, State>,
    ) {
        use zwlr_output_configuration_head_v1::{Error, Request};
        let mut config = data.state.lock().unwrap();
        let Some(change) = config.heads.iter_mut().find(|h| h.output == data.output) else { return };
        match request {
            Request::SetMode { mode } => {
                if let Some(mode) = mode.data::<Mode>() {
                    change.mode = Some(*mode);
                }
            }
            Request::SetCustomMode { width, height, refresh } => {
                // Only modes the display offers; the nearest refresh rate wins.
                let wanted = refresh;
                let best = change
                    .output
                    .modes()
                    .into_iter()
                    .filter(|m| m.size.w == width && m.size.h == height)
                    .min_by_key(|m| (m.refresh - wanted).abs());
                match best {
                    Some(mode) => change.mode = Some(mode),
                    None => resource.post_error(Error::InvalidCustomMode, "the display has no such mode"),
                }
            }
            Request::SetPosition { x, y } => change.position = Some((x, y).into()),
            Request::SetTransform { transform } => match transform.into_result().ok().and_then(transform_from) {
                Some(transform) => change.transform = Some(transform),
                None => resource.post_error(Error::InvalidTransform, "unknown transform"),
            },
            Request::SetScale { scale } => {
                if scale > 0.0 {
                    change.scale = Some(scale);
                } else {
                    resource.post_error(Error::InvalidScale, "scale must be positive");
                }
            }
            _ => {}
        }
    }
}
