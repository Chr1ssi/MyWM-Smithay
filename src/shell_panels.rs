//! The panels the bar's buttons open (`bar.qml`'s details panel): one at a time, below the bar of the monitor
//! whose button was clicked. A click into a panel gives it the keyboard (Escape closes it), as its window got it on
//! demand; a button pressed anywhere but the panel and the bar closes it, as the window did when it lost the
//! keyboard. The wheel scrolls the notification list where it is over it, else the panel's area.
use mywm_shell_protocol::{Command, PlayerAction};
use mywm_ui::{
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
    painted: Option<Painted<PanelTarget>>,
    max_scroll: f32,
    max_list_scroll: f32,
}

impl State {
    /// A panel button: open its panel on `output`, or close it if it is open there.
    pub fn panel_toggle(&mut self, kind: Panel, output: Output) {
        if self.shell.panel.as_ref().is_some_and(|p| p.kind == kind && p.output == output) {
            return self.panel_close();
        }
        self.panel_close();
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
        });
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
            PanelTarget::Card(CardTarget::Card(_)) | PanelTarget::Background | PanelTarget::List => {}
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

    fn panel_content(&self, kind: Panel, list_scroll: f32) -> ContentModel {
        match kind {
            Panel::Media => ContentModel::Media { players: self.player_views() },
            Panel::Notifications => ContentModel::Notifications { items: self.notification_views(), list_scroll },
            // Not opened yet (see `bar_clicked`): their contents come with their own steps.
            Panel::Audio | Panel::Control | Panel::Calendar => ContentModel::Media { players: Vec::new() },
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
        let Some((kind, list_scroll, output)) = self.shell.panel.as_ref().map(|p| (p.kind, p.list_scroll, p.output.clone())) else { return };
        let Some(geo) = self.space.output_geometry(&output) else { return };
        let hover = match self.shell.hover {
            Some(ShellHit::Panel(target)) => Some(target),
            _ => None,
        };
        let content = self.panel_content(kind, list_scroll);
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

    fn panel_origin(&self) -> Option<(f64, f64)> {
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
