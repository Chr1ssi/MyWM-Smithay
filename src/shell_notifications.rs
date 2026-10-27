//! Notifications in the built-in shell, as splice-shell's Quickshell bar kept them: a list (newest first) behind
//! the bell, which opens the notification center (see `shell_panels`), and toasts at the top right of the first
//! monitor.
//!
//! Toasts are at most four. One hides after its own timeout, or after 4 seconds (low urgency) or 7 seconds
//! (else) without one; with a timeout of 0 it stays. The pointer over a toast holds it, and its time starts
//! over when the pointer leaves (a QML `Timer` that stops and runs again). While a game shows fullscreen only
//! critical notifications pop up; the rest wait in the list. A hidden toast stays in the list.
use std::time::{Duration, Instant};

use splice_shell_protocol::{Command, Notification, Urgency};
use splice_ui::{markup, notifications::NotificationView, scene::Picture};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::{State, shell::ShellHit};

const MAX_TOASTS: usize = 4;

struct Note {
    view: NotificationView,
    urgency: Urgency,
    expire_timeout: i32,
}

struct Toast {
    id: u32,
    /// When it hides; `None` while the pointer holds it or if it stays.
    deadline: Option<Instant>,
}

#[derive(Default)]
pub struct Notes {
    /// Newest first.
    list: Vec<Note>,
    /// Newest first.
    toasts: Vec<Toast>,
    /// The toast the pointer holds.
    held: Option<u32>,
    /// Tells the timer for the next toast to hide apart from earlier ones.
    timer: u64,
}

impl Note {
    /// How long its toast shows; `None`: it stays.
    fn interval(&self) -> Option<Duration> {
        match self.expire_timeout {
            0 => None,
            ms if ms > 0 => Some(Duration::from_millis(ms as u64)),
            _ if self.urgency == Urgency::Low => Some(Duration::from_millis(4000)),
            _ => Some(Duration::from_millis(7000)),
        }
    }
}

impl State {
    /// A notification came, or new contents for one shown already.
    pub fn shell_notification(&mut self, notification: Notification) {
        let picture = notification.image.as_ref().and_then(|image| Picture::from_rgba(image.width, image.height, &image.rgba));
        let view = NotificationView {
            id: notification.id,
            summary: notification.summary,
            app_name: notification.app_name,
            body: markup::parse(&notification.body),
            critical: notification.urgency == Urgency::Critical,
            actions: notification.actions.into_iter().map(|a| (a.identifier, a.text)).collect(),
            picture,
        };
        let note = Note { view, urgency: notification.urgency, expire_timeout: notification.expire_timeout };
        let gaming = self.game_shown();
        let notes = &mut self.shell.notes;
        // Replaced contents keep their place and their toast's time, as Quickshell's updated notification does.
        if let Some(existing) = notes.list.iter_mut().find(|n| n.view.id == note.view.id) {
            *existing = note;
        } else {
            let pops_up = !gaming || note.urgency == Urgency::Critical;
            if pops_up {
                let deadline = note.interval().map(|interval| Instant::now() + interval);
                notes.toasts.insert(0, Toast { id: note.view.id, deadline });
                notes.toasts.truncate(MAX_TOASTS);
            }
            notes.list.insert(0, note);
        }
        self.toasts_arm();
        self.shell_changed();
    }

    /// The notification is gone (closed by its app, or by one of its actions).
    pub fn shell_notification_closed(&mut self, id: u32) {
        let notes = &mut self.shell.notes;
        notes.list.retain(|n| n.view.id != id);
        notes.toasts.retain(|t| t.id != id);
        self.shell_changed();
    }

    /// The services process ended: what it served is gone with it.
    pub fn shell_notifications_reset(&mut self) {
        let notes = &mut self.shell.notes;
        if !notes.list.is_empty() || !notes.toasts.is_empty() {
            notes.list.clear();
            notes.toasts.clear();
            self.shell_changed();
        }
    }

    pub fn notification_count(&self) -> usize {
        self.shell.notes.list.len()
    }

    /// The toasts to show, newest first.
    pub fn toast_views(&self) -> Vec<NotificationView> {
        let notes = &self.shell.notes;
        notes.toasts.iter().filter_map(|t| notes.list.iter().find(|n| n.view.id == t.id)).map(|n| n.view.clone()).collect()
    }

    /// While a game shows, only critical toasts stay up.
    pub fn toasts_for_gaming(&mut self) {
        if !self.game_shown() {
            return;
        }
        let notes = &mut self.shell.notes;
        let before = notes.toasts.len();
        let list = &notes.list;
        notes.toasts.retain(|t| list.iter().any(|n| n.view.id == t.id && n.urgency == Urgency::Critical));
        if notes.toasts.len() != before {
            self.shell.dirty = true;
        }
    }

    /// The pointer is over `hit`: a toast under it is held, one it left starts its time again.
    pub fn toasts_hovered(&mut self, hit: Option<ShellHit>) {
        let held = match hit {
            Some(ShellHit::Notices(splice_ui::notices::NoticeTarget::Toast(target))) => Some(target.id()),
            _ => None,
        };
        let notes = &mut self.shell.notes;
        if held == notes.held {
            return;
        }
        let now = Instant::now();
        let left = std::mem::replace(&mut notes.held, held);
        if let Some(left) = left
            && let Some(note) = notes.list.iter().find(|n| n.view.id == left)
            && let Some(toast) = notes.toasts.iter_mut().find(|t| t.id == left)
        {
            toast.deadline = note.interval().map(|interval| now + interval);
        }
        if let Some(held) = held
            && let Some(toast) = notes.toasts.iter_mut().find(|t| t.id == held)
        {
            toast.deadline = None;
        }
        self.toasts_arm();
    }

    /// Wake when the next toast is due to hide.
    fn toasts_arm(&mut self) {
        let notes = &mut self.shell.notes;
        notes.timer += 1;
        let Some(next) = notes.toasts.iter().filter_map(|t| t.deadline).min() else { return };
        let generation = notes.timer;
        let registered = self.loop_handle.insert_source(Timer::from_deadline(next), move |_, _, state: &mut State| {
            if state.shell.notes.timer == generation {
                let now = Instant::now();
                // Only the toast goes; the notification stays in the list.
                state.shell.notes.toasts.retain(|t| t.deadline.is_none_or(|deadline| deadline > now));
                state.shell_changed();
                state.toasts_arm();
            }
            TimeoutAction::Drop
        });
        if let Err(error) = registered {
            tracing::warn!("toasts cannot time out: {}", error.error);
        }
    }

    /// The user closed a notification (its card's "×").
    pub fn notification_dismiss(&mut self, id: u32) {
        self.shell_notification_closed(id);
        self.services_send(Command::Dismiss { id });
    }

    /// The user chose an action of a notification; a non-resident one closes (the services say so).
    pub fn notification_invoke(&mut self, id: u32, index: usize) {
        let action = self.shell.notes.list.iter().find(|n| n.view.id == id).and_then(|n| n.view.actions.get(index)).map(|a| a.0.clone());
        if let Some(action) = action {
            self.services_send(Command::Invoke { id, action });
        }
    }

    /// "Clear all" in the notification center.
    pub fn notifications_clear(&mut self) {
        let ids: Vec<u32> = self.shell.notes.list.iter().map(|n| n.view.id).collect();
        for id in ids {
            self.notification_dismiss(id);
        }
    }

    /// The notifications as the notification center lists them, newest first.
    pub fn notification_views(&self) -> Vec<NotificationView> {
        self.shell.notes.list.iter().map(|n| n.view.clone()).collect()
    }
}
