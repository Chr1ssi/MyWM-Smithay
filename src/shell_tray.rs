//! The tray in the built-in shell: the items that are not passive show in the bar; a left click activates one (or
//! opens its menu if it has only a menu), the middle button activates it otherwise, the right button opens its
//! menu, the wheel scrolls it. The menu opens below the bar at the right edge (one at a time, closing the panels);
//! a click into it gives it the keyboard, a click elsewhere closes it.
use splice_shell_protocol::{Command, MenuEntry, Toggle, TrayItem, TrayStatus};
use splice_ui::{
    bar::TrayIcon,
    markup,
    scene::Picture,
    text::Fonts,
    tray_menu::{self, EntryView, MenuTarget, TrayMenuModel},
};
use smithay::{backend::renderer::gles::GlesRenderer, output::Output};

use crate::{
    State,
    render::OutputElement,
    shell::{Painted, ShellHit, element, paint},
};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

/// A tray item as the services process reported it, its icon decoded.
pub struct TrayEntry {
    pub item: TrayItem,
    pub picture: Option<Picture>,
}

/// A menu entry with its icon decoded.
struct MenuNode {
    id: i32,
    view: EntryView,
    children: Vec<MenuNode>,
}

impl MenuNode {
    fn from(entry: MenuEntry) -> Self {
        let picture = entry.icon.and_then(|image| Picture::from_rgba(image.width, image.height, &image.rgba));
        let toggle = match entry.toggle {
            Toggle::None => tray_menu::Toggle::None,
            Toggle::Check => tray_menu::Toggle::Check,
            Toggle::Radio => tray_menu::Toggle::Radio,
        };
        let view = EntryView {
            label: markup::mnemonic(&entry.label),
            separator: entry.separator,
            enabled: entry.enabled,
            toggle,
            checked: entry.checked,
            icon_name: entry.icon_name,
            picture,
            submenu: entry.submenu,
        };
        Self { id: entry.id, view, children: entry.children.into_iter().map(MenuNode::from).collect() }
    }
}

pub struct TrayMenuOpen {
    output: Output,
    address: String,
    title: String,
    /// The menu, once the services process has read it.
    root: Option<MenuNode>,
    /// Indices of the submenus shown, from the root.
    path: Vec<usize>,
    focused: bool,
    scroll: f32,
    max_scroll: f32,
    model: Option<TrayMenuModel>,
    painted: Option<Painted<MenuTarget>>,
}

impl TrayMenuOpen {
    /// The menu shown (the root or the submenu `path` leads to).
    fn shown(&self) -> Option<&MenuNode> {
        self.path.iter().try_fold(self.root.as_ref()?, |node, index| node.children.get(*index))
    }
}

impl State {
    /// The tray items the bar shows: those that are not passive.
    fn tray_shown(&self) -> impl Iterator<Item = &TrayEntry> {
        self.shell.services.tray.iter().filter(|e| e.item.status != TrayStatus::Passive)
    }

    pub fn tray_icons(&self) -> Vec<TrayIcon> {
        self.tray_shown().map(|e| TrayIcon { picture: e.picture.clone(), attention: e.item.status == TrayStatus::NeedsAttention }).collect()
    }

    /// New tray items from the services process; a menu of an item that went closes.
    pub fn shell_tray(&mut self, items: Vec<TrayItem>) {
        self.shell.services.tray = items
            .into_iter()
            .map(|mut item| {
                let picture = item.icon.take().and_then(|image| Picture::from_rgba(image.width, image.height, &image.rgba));
                TrayEntry { item, picture }
            })
            .collect();
        let open = self.shell.tray_menu.as_ref().map(|m| m.address.clone());
        if open.is_some_and(|address| !self.shell.services.tray.iter().any(|e| e.item.address == address)) {
            self.tray_menu_close();
        }
        self.shell_changed();
    }

    /// The menu of a tray item, read for the menu that is open.
    pub fn shell_tray_menu(&mut self, address: String, menu: MenuEntry) {
        let Some(open) = self.shell.tray_menu.as_mut().filter(|m| m.address == address) else { return };
        open.root = Some(MenuNode::from(menu));
        // A submenu that is gone falls back to where the path still leads.
        while !open.path.is_empty() && open.shown().is_none() {
            open.path.pop();
        }
        self.shell_changed();
    }

    /// A button released on the tray icon at `index` of those the bar shows.
    pub fn tray_clicked(&mut self, monitor: usize, index: usize, button: u32) {
        let Some(entry) = self.tray_shown().nth(index) else { return };
        let item = entry.item.clone();
        match button {
            BTN_MIDDLE => self.services_send(Command::TraySecondaryActivate { address: item.address }),
            BTN_RIGHT => self.tray_menu_open(monitor, &item),
            BTN_LEFT if item.only_menu => self.tray_menu_open(monitor, &item),
            BTN_LEFT => self.services_send(Command::TrayActivate { address: item.address }),
            _ => {}
        }
    }

    /// The wheel over the tray icon at `index`: `steps` notches, positive away from the user.
    pub fn tray_scroll(&mut self, index: usize, steps: f32) {
        let Some(address) = self.tray_shown().nth(index).map(|e| e.item.address.clone()) else { return };
        self.services_send(Command::TrayScroll { address, delta: (steps * 120.0).round() as i32, horizontal: false });
    }

    fn tray_menu_open(&mut self, monitor: usize, item: &TrayItem) {
        if !item.has_menu {
            return;
        }
        let Some(output) = self.outputs.get(monitor).map(|e| e.output.clone()) else { return };
        self.panel_close();
        self.tray_menu_close();
        let title = tray_menu::title(&item.title, &item.id, self.shell.language);
        self.shell.tray_menu = Some(TrayMenuOpen {
            output,
            address: item.address.clone(),
            title,
            root: None,
            path: Vec::new(),
            focused: false,
            scroll: 0.0,
            max_scroll: 0.0,
            model: None,
            painted: None,
        });
        self.services_send(Command::TrayMenu { address: item.address.clone() });
        self.shell_changed();
    }

    pub fn tray_menu_close(&mut self) {
        if let Some(menu) = self.shell.tray_menu.take() {
            self.queue_redraw_output(&menu.output);
            self.shell_changed();
            if menu.focused {
                self.sync_focus();
            }
        }
    }

    pub fn tray_menu_on(&self, output: &Output) -> bool {
        self.shell.tray_menu.as_ref().is_some_and(|m| &m.output == output)
    }

    pub fn tray_menu_takes_keyboard(&self) -> bool {
        self.shell.tray_menu.as_ref().is_some_and(|m| m.focused)
    }

    /// A click on the menu's `target`.
    pub fn tray_menu_clicked(&mut self, target: MenuTarget) {
        let Some(menu) = &mut self.shell.tray_menu else { return };
        if !menu.focused {
            menu.focused = true;
            self.sync_focus();
        }
        let Some(menu) = &mut self.shell.tray_menu else { return };
        match target {
            MenuTarget::Back => {
                menu.path.pop();
                menu.scroll = 0.0;
                self.shell_changed();
            }
            MenuTarget::Entry(index) => {
                let Some(node) = menu.shown().and_then(|shown| shown.children.get(index)) else { return };
                let (id, submenu) = (node.id, node.view.submenu);
                let address = menu.address.clone();
                if submenu {
                    menu.path.push(index);
                    menu.scroll = 0.0;
                    self.services_send(Command::TrayMenuShow { address, id });
                    self.shell_changed();
                } else {
                    self.services_send(Command::TrayMenuClick { address, id });
                    self.tray_menu_close();
                }
            }
            MenuTarget::Background | MenuTarget::List => {}
        }
    }

    pub fn tray_menu_scroll(&mut self, steps: f32) {
        if let Some(menu) = &mut self.shell.tray_menu {
            let scroll = (menu.scroll - steps * splice_ui::panel::SCROLL_STEP).clamp(0.0, menu.max_scroll);
            if scroll != menu.scroll {
                menu.scroll = scroll;
                self.shell_changed();
            }
        }
    }

    /// Paint the menu if what it shows changed.
    pub fn tray_menu_refresh(&mut self, fonts: &mut Fonts, redraw: &mut Vec<Output>) {
        let Some(geo) = self.shell.tray_menu.as_ref().and_then(|m| self.space.output_geometry(&m.output)) else { return };
        let hover = match self.shell.hover {
            Some(ShellHit::TrayMenu(target)) => Some(target),
            _ => None,
        };
        let theme = self.shell.theme.clone();
        let Some(menu) = &mut self.shell.tray_menu else { return };
        // Until the menu is read nothing shows, as an empty menu would flash.
        let Some(shown) = menu.shown() else { return };
        let entries = shown.children.iter().map(|child| child.view.clone()).collect();
        let model = TrayMenuModel {
            output_width: geo.size.w as f32,
            title: menu.title.clone(),
            entries,
            can_go_back: !menu.path.is_empty(),
            scroll: menu.scroll,
            hover,
        };
        if menu.model.as_ref() == Some(&model) {
            return;
        }
        let (scene, max_scroll) = tray_menu::scene(&model, &theme, fonts);
        menu.max_scroll = max_scroll;
        menu.scroll = menu.scroll.min(max_scroll);
        menu.painted = paint(scene, fonts, menu.output.current_scale().fractional_scale());
        menu.model = Some(model);
        redraw.push(menu.output.clone());
    }

    pub fn tray_menu_repaint(&mut self) {
        if let Some(menu) = &mut self.shell.tray_menu {
            menu.model = None;
        }
    }

    fn tray_menu_origin(&self) -> Option<(f64, f64)> {
        let menu = self.shell.tray_menu.as_ref()?;
        let geo = self.space.output_geometry(&menu.output)?;
        let width = menu.painted.as_ref()?.scene.width;
        Some((f64::from(geo.size.w as f32 - tray_menu::RIGHT - width), f64::from(tray_menu::TOP)))
    }

    pub fn tray_menu_elements(&self, renderer: &mut GlesRenderer, output: &Output) -> Option<OutputElement> {
        let menu = self.shell.tray_menu.as_ref().filter(|m| &m.output == output)?;
        element(renderer, menu.painted.as_ref()?, self.tray_menu_origin()?, output.current_scale().fractional_scale())
    }

    pub fn tray_menu_at_pointer(&self) -> Option<ShellHit> {
        let menu = self.shell.tray_menu.as_ref()?;
        let geo = self.space.output_geometry(&menu.output)?;
        let (x, y) = self.tray_menu_origin()?;
        let local = self.pointer_location - geo.loc.to_f64();
        menu.painted.as_ref()?.scene.target_at((local.x - x) as f32, (local.y - y) as f32).map(ShellHit::TrayMenu)
    }
}
