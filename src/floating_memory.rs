//! Where each app's floating window was last: a window of the app that opens floating starts there again,
//! also after a restart (`$XDG_STATE_HOME/mywm/floating`). Dialogs are left out; they open over their parent.
use std::{collections::HashMap, path::PathBuf};

use mywm_layout::Rect;

use crate::{State, desktop::WindowId};

/// One line per app: `x y width height app_id`, the rect relative to the monitor's work area.
fn path() -> Option<PathBuf> {
    Some(mywm_config::state_dir()?.join("floating"))
}

pub fn load() -> HashMap<String, Rect> {
    let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else { return HashMap::new() };
    text.lines()
        .filter_map(|line| {
            let mut fields = line.splitn(5, ' ');
            let mut number = || fields.next()?.parse::<i32>().ok();
            let rect = Rect { x: number()?, y: number()?, width: number()?, height: number()? };
            let app_id = fields.next()?.trim();
            (!app_id.is_empty() && rect.width > 0 && rect.height > 0).then(|| (app_id.to_owned(), rect))
        })
        .collect()
}

fn save(memory: &HashMap<String, Rect>) {
    let Some(path) = path() else { return };
    let mut entries: Vec<_> = memory.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let text: String = entries.iter().map(|(app, r)| format!("{} {} {} {} {app}\n", r.x, r.y, r.width, r.height)).collect();
    // Written beside and renamed over, so a crash never leaves half a file.
    let partial = path.with_extension("new");
    let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&partial, text));
    if let Err(error) = written.and_then(|()| std::fs::rename(&partial, &path)) {
        tracing::warn!("cannot remember floating windows in {}: {error}", path.display());
    }
}

impl State {
    /// Remember where `id` floats, if it is an app's own floating window (not a dialog).
    pub fn remember_floating(&mut self, id: WindowId) {
        let Some(m) = self.desktop.get(id) else { return };
        let (Some(app_id), Some(rect)) = (m.app_id.clone(), m.floating_rect) else { return };
        if !m.floating || m.parent.is_some() || m.dialog || self.floating_memory.get(&app_id) == Some(&rect) {
            return;
        }
        self.floating_memory.insert(app_id, rect);
        save(&self.floating_memory);
    }

    /// Where a new floating window of `app_id` starts, if one of the app floated before.
    pub fn recall_floating(&self, app_id: Option<&str>) -> Option<Rect> {
        self.floating_memory.get(app_id?).copied()
    }
}
