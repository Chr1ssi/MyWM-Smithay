//! The panels the bar's buttons open (`bar.qml`'s details panel): one at a time, below the bar of the monitor
//! whose button was clicked. A click into a panel gives it the keyboard (Escape closes it), as its window got it on
//! demand; a button pressed anywhere but the panel and the bar closes it, as the window did when it lost the
//! keyboard. The wheel scrolls the notification list where it is over it, else the panel's area.
use mywm_shell_protocol::{Command, NodeKind, PlayerAction};
use mywm_ui::{
    audio::{self, AudioModel, AudioTarget, NodeView},
    bar::Panel,
    media::{MediaTarget, PlayerView},
    notifications::CardTarget,
    panel::{self, ContentModel, PanelModel, PanelTarget, SCROLL_STEP},
    text::Fonts,
};
use smithay::{backend::renderer::gles::GlesRenderer, output::Output};

use crate::{
    State,
    render::OutputElement,
    shell::{Painted, ShellHit, element, paint},
};

pub struct OpenPanel {
    pub kind: Panel,
    pub output: Output,
    scroll: f32,
    list_scroll: f32,
    /// It was clicked and takes the keyboard.
    focused: bool,
    model: Option<PanelModel>,
    pub painted: Option<Painted<PanelTarget>>,
    max_scroll: f32,
    max_list_scroll: f32,
    /// The audio panel shows the other outputs or microphones to choose from.
    outputs_expanded: bool,
    inputs_expanded: bool,
}

impl State {
    /// A panel button: open its panel on `output`, or close it if it is open there.
    pub fn panel_toggle(&mut self, kind: Panel, output: Output) {
        if self.shell.panel.as_ref().is_some_and(|p| p.kind == kind && p.output == output) {
            return self.panel_close();
        }
        self.panel_close();
        self.tray_menu_close();
        self.shell.panel = Some(OpenPanel {
            kind,
            output,
            scroll: 0.0,
            list_scroll: 0.0,
            focused: false,
            model: None,
            painted: None,
            max_scroll: 0.0,
            max_list_scroll: 0.0,
            outputs_expanded: false,
            inputs_expanded: false,
        });
        if kind == Panel::Control {
            self.addresses_follow();
        }
        self.shell_changed();
    }

    /// The panel open on `output`.
    pub fn panel_on(&self, output: &Output) -> Option<Panel> {
        self.shell.panel.as_ref().filter(|p| &p.output == output).map(|p| p.kind)
    }

    pub fn panel_close(&mut self) {
        if let Some(panel) = self.shell.panel.take() {
            self.queue_redraw_output(&panel.output);
            self.shell_changed();
            if panel.focused {
                self.sync_focus();
            }
        }
    }

    /// A button went down somewhere else than the panel or the bar.
    pub fn panel_pressed_elsewhere(&mut self) {
        if self.shell.panel.is_some() {
            self.panel_close();
        }
        if self.shell.tray_menu.is_some() {
            self.tray_menu_close();
        }
    }

    pub fn panel_takes_keyboard(&self) -> bool {
        self.shell.panel.as_ref().is_some_and(|p| p.focused)
    }

    fn panel_focus(&mut self) {
        if let Some(panel) = &mut self.shell.panel
            && !panel.focused
        {
            panel.focused = true;
            self.sync_focus();
        }
    }

    /// A click on the open panel's `target`.
    pub fn panel_clicked(&mut self, target: PanelTarget) {
        self.panel_focus();
        match target {
            PanelTarget::ClearAll => self.notifications_clear(),
            PanelTarget::Card(CardTarget::Dismiss(id)) => self.notification_dismiss(id),
            PanelTarget::Card(CardTarget::Action(id, index)) => self.notification_invoke(id, index),
            PanelTarget::Media(target) => self.media_clicked(target),
            PanelTarget::Audio(target) => self.audio_clicked(target),
            PanelTarget::Control(target) => self.control_clicked(target),
            PanelTarget::Card(CardTarget::Card(_)) | PanelTarget::Background | PanelTarget::List => {}
        }
    }

    fn audio_clicked(&mut self, target: AudioTarget) {
        match target {
            AudioTarget::Select(id) => {
                let kind = self.shell.services.status.nodes.iter().find(|n| n.id == id).map(|n| n.kind);
                if let Some(panel) = &mut self.shell.panel {
                    match kind {
                        Some(NodeKind::Sink) => panel.outputs_expanded = false,
                        Some(NodeKind::Source) => panel.inputs_expanded = false,
                        _ => {}
                    }
                }
                self.services_send(Command::SetDefault { id });
                self.shell_changed();
            }
            AudioTarget::Mute(id) => self.node_toggle_mute(id),
            AudioTarget::ToggleOutputs | AudioTarget::ToggleInputs => {
                if let Some(panel) = &mut self.shell.panel {
                    let expanded = if target == AudioTarget::ToggleOutputs { &mut panel.outputs_expanded } else { &mut panel.inputs_expanded };
                    *expanded = !*expanded;
                }
                self.shell_changed();
            }
            // Pressing and dragging sets it (see `slider_dragged`).
            AudioTarget::Slider(_) => {}
        }
    }

    /// The pointer holds the slider of `shell.dragging`: its volume follows the pointer.
    pub fn slider_dragged(&mut self) {
        let Some(id) = self.shell.dragging else { return };
        let Some(panel) = self.shell.panel.as_ref() else { return };
        let (Some(geo), Some((x, _))) = (self.space.output_geometry(&panel.output), self.panel_origin()) else { return };
        let slider = panel.painted.as_ref().and_then(|p| p.scene.target_rect(&PanelTarget::Audio(AudioTarget::Slider(id))));
        let Some(slider) = slider else { return };
        let local = (self.pointer_location.x - f64::from(geo.loc.x) - x) as f32;
        let volume = audio::slider_value(slider, local);
        let current = self.shell.services.status.nodes.iter().find(|n| n.id == id).and_then(|n| n.audio).map(|a| a.volume);
        if current.is_none_or(|v| (v - volume).abs() >= 0.001) {
            self.node_set_volume(id, volume);
        }
    }

    fn audio_model(&self, outputs_expanded: bool, inputs_expanded: bool) -> AudioModel {
        let status = &self.shell.services.status;
        let views = |kind: NodeKind| -> Vec<NodeView> {
            status
                .nodes
                .iter()
                .filter(|n| n.kind == kind)
                .map(|n| {
                    let label = [&n.application, &n.description, &n.name];
                    let label = label.into_iter().find(|l| !l.is_empty()).cloned().unwrap_or_default();
                    NodeView { id: n.id, label, audio: n.audio.map(|a| (a.volume, a.muted)) }
                })
                .collect()
        };
        AudioModel {
            outputs: views(NodeKind::Sink),
            inputs: views(NodeKind::Source),
            applications: views(NodeKind::Stream),
            default_output: status.default_sink,
            default_input: status.default_source,
            outputs_expanded,
            inputs_expanded,
        }
    }

    fn media_clicked(&mut self, target: MediaTarget) {
        let (index, action) = match target {
            MediaTarget::Previous(index) => (index, PlayerAction::Previous),
            MediaTarget::Next(index) => (index, PlayerAction::Next),
            MediaTarget::Toggle(index) => {
                let playing = self.shell.services.players.get(index).is_some_and(|p| p.player.playing);
                (index, if playing { PlayerAction::Pause } else { PlayerAction::Play })
            }
        };
        if let Some(entry) = self.shell.services.players.get(index) {
            let bus = entry.player.bus.clone();
            self.services_send(Command::Player { bus, action });
        }
    }

    /// The wheel over the panel's `target`.
    pub fn panel_scroll(&mut self, target: &PanelTarget, steps: f32) {
        let Some(panel) = &mut self.shell.panel else { return };
        let (scroll, max) = match target {
            PanelTarget::List | PanelTarget::Card(_) if panel.max_list_scroll > 0.0 => (&mut panel.list_scroll, panel.max_list_scroll),
            _ => (&mut panel.scroll, panel.max_scroll),
        };
        let moved = (*scroll - steps * SCROLL_STEP).clamp(0.0, max);
        if moved != *scroll {
            *scroll = moved;
            self.shell_changed();
        }
    }

    fn panel_content(&self, panel: &OpenPanel) -> ContentModel {
        match panel.kind {
            Panel::Media => ContentModel::Media { players: self.player_views() },
            Panel::Audio => ContentModel::Audio(self.audio_model(panel.outputs_expanded, panel.inputs_expanded)),
            Panel::Control => ContentModel::Control(self.control_model()),
            Panel::Notifications => ContentModel::Notifications { items: self.notification_views(), list_scroll: panel.list_scroll },
            // Not opened yet (see `bar_clicked`): its content comes with its own step.
            Panel::Calendar => ContentModel::Media { players: Vec::new() },
        }
    }

    fn player_views(&self) -> Vec<PlayerView> {
        self.shell
            .services
            .players
            .iter()
            .map(|entry| {
                let p = &entry.player;
                PlayerView {
                    identity: p.identity.clone(),
                    title: p.title.clone(),
                    artist: p.artist.clone(),
                    art: entry.cover.clone(),
                    playing: p.playing,
                    can_previous: p.can_previous,
                    can_toggle: p.can_toggle,
                    can_next: p.can_next,
                }
            })
            .collect()
    }

    /// Paint the open panel if what it shows changed; its output goes to `redraw`.
    pub fn panel_refresh(&mut self, fonts: &mut Fonts, redraw: &mut Vec<Output>) {
        let Some((kind, output)) = self.shell.panel.as_ref().map(|p| (p.kind, p.output.clone())) else { return };
        let Some(geo) = self.space.output_geometry(&output) else { return };
        let hover = match self.shell.hover {
            Some(ShellHit::Panel(target)) => Some(target),
            _ => None,
        };
        let Some(content) = self.shell.panel.as_ref().map(|panel| self.panel_content(panel)) else { return };
        let language = self.shell.language;
        let theme = self.shell.theme.clone();
        let Some(panel) = &mut self.shell.panel else { return };
        let model = PanelModel { kind, output: (geo.size.w as f32, geo.size.h as f32), content, scroll: panel.scroll, hover, language };
        if panel.model.as_ref() == Some(&model) {
            return;
        }
        let scene = panel::scene(&model, &theme, fonts);
        panel.max_scroll = scene.max_scroll;
        panel.max_list_scroll = scene.max_list_scroll;
        panel.scroll = panel.scroll.min(scene.max_scroll);
        panel.list_scroll = panel.list_scroll.min(scene.max_list_scroll);
        panel.painted = paint(scene.scene, fonts, output.current_scale().fractional_scale());
        panel.model = Some(model);
        redraw.push(output);
    }

    /// Forget the panel's picture (the theme changed).
    pub fn panel_repaint(&mut self) {
        if let Some(panel) = &mut self.shell.panel {
            panel.model = None;
        }
    }

    pub fn panel_origin(&self) -> Option<(f64, f64)> {
        let panel = self.shell.panel.as_ref()?;
        let geo = self.space.output_geometry(&panel.output)?;
        let (x, y) = panel::origin(panel.kind, geo.size.w as f32, panel.painted.as_ref()?.scene.width);
        Some((f64::from(x), f64::from(y)))
    }

    /// The panel on `output`.
    pub fn panel_elements(&self, renderer: &mut GlesRenderer, output: &Output) -> Option<OutputElement> {
        let panel = self.shell.panel.as_ref().filter(|p| &p.output == output)?;
        element(renderer, panel.painted.as_ref()?, self.panel_origin()?, output.current_scale().fractional_scale())
    }

    /// The panel at the pointer.
    pub fn panel_at_pointer(&self) -> Option<ShellHit> {
        let panel = self.shell.panel.as_ref()?;
        let geo = self.space.output_geometry(&panel.output)?;
        let (x, y) = self.panel_origin()?;
        let local = self.pointer_location - geo.loc.to_f64();
        panel.painted.as_ref()?.scene.target_at((local.x - x) as f32, (local.y - y) as f32).map(ShellHit::Panel)
    }
}
