//! Workspace ownership across monitors: one fixed workspace per monitor,
//! dynamically created extras that live on the monitor they were created on, and
//! a gaming workspace that exists only while a game runs. Ported from the
//! River-based MyWM, independent of any compositor types.
use std::collections::HashSet;

use crate::{GAMING, Kind, MAX_NUMBER, Rect, Workspaces};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug)]
pub struct Monitor<T> {
    /// Connector name (`DP-3`), matched against `workspace_outputs`.
    pub name: Option<String>,
    /// Position and size in the global logical coordinate space.
    pub area: Rect,
    pub workspaces: Workspaces<T>,
}

#[derive(Debug)]
pub struct Desk<T> {
    pub monitors: Vec<Monitor<T>>,
    /// Monitors in workspace order: the first gets workspace 1, the next 2, ...
    pub workspace_outputs: Vec<String>,
    pub gaming_output: Option<String>,
    /// Workspaces of unplugged monitors while no monitor is left.
    detached: Option<Workspaces<T>>,
}

impl<T: Clone + Eq> Desk<T> {
    pub fn new(workspace_outputs: Vec<String>, gaming_output: Option<String>) -> Self {
        Self { monitors: Vec::new(), workspace_outputs, gaming_output, detached: None }
    }

    /// Number of the fixed workspace of `monitor`: its position in `workspace_outputs`,
    /// or the next free number for monitors not listed there.
    pub fn home_number(&self, monitor: usize) -> usize {
        let listed = &self.workspace_outputs;
        let position = |index: usize| {
            self.monitors[index]
                .name
                .as_ref()
                .and_then(|name| listed.iter().position(|item| item == name))
        };
        if let Some(position) = position(monitor) {
            return position + 1;
        }
        listed.len() + (0..monitor).filter(|index| position(*index).is_none()).count() + 1
    }

    /// The monitor showing the gaming workspace: `gaming_output`, else the monitor with workspace 1.
    pub fn gaming_monitor(&self) -> Option<usize> {
        self.gaming_output
            .as_ref()
            .and_then(|wanted| self.monitors.iter().position(|m| m.name.as_ref() == Some(wanted)))
            .or_else(|| (0..self.monitors.len()).find(|index| self.home_number(*index) == 1))
            .or_else(|| (!self.monitors.is_empty()).then_some(0))
    }

    /// The monitor a workspace number lives on, if that workspace exists. The gaming
    /// workspace always belongs to the gaming monitor.
    pub fn owner(&self, number: usize) -> Option<usize> {
        if number == GAMING {
            return self.gaming_monitor();
        }
        self.monitors.iter().position(|m| m.workspaces.contains(number))
    }

    fn used_numbers(&self) -> HashSet<usize> {
        let mut used: HashSet<_> = (1..=self.workspace_outputs.len()).collect();
        for (index, monitor) in self.monitors.iter().enumerate() {
            used.insert(self.home_number(index));
            used.extend(monitor.workspaces.numbers());
        }
        used
    }

    /// Lowest regular workspace number not used by any monitor.
    pub fn free_number(&self) -> Option<usize> {
        let used = self.used_numbers();
        (1..=MAX_NUMBER).find(|number| !used.contains(number))
    }

    /// Create an extra workspace on `monitor` (or reuse its empty one) and return its number.
    pub fn create(&mut self, monitor: usize) -> Option<usize> {
        if let Some(empty) = self.monitors[monitor]
            .workspaces
            .entries
            .iter()
            .find(|entry| entry.kind == Kind::Extra && entry.windows.is_empty())
        {
            return Some(empty.number);
        }
        let number = self.free_number()?;
        self.monitors[monitor].workspaces.ensure(number, Kind::Extra);
        Some(number)
    }

    /// Make sure the gaming workspace exists on the gaming monitor; returns that monitor.
    pub fn ensure_gaming(&mut self) -> Option<usize> {
        let monitor = self.gaming_monitor()?;
        self.monitors[monitor].workspaces.ensure(GAMING, Kind::Gaming);
        Some(monitor)
    }

    /// Add a monitor; returns its index. Workspaces of unplugged monitors return to it.
    pub fn add_monitor(&mut self, name: Option<String>, area: Rect) -> usize {
        let mut workspaces = Workspaces::new(1);
        if let Some(detached) = self.detached.take() {
            workspaces.absorb(detached);
        }
        self.monitors.push(Monitor { name, area, workspaces });
        self.reconcile();
        self.monitors.len() - 1
    }

    /// Remove a monitor; its workspaces move to the first remaining one (or wait for a new monitor).
    pub fn remove_monitor(&mut self, index: usize) {
        let removed = self.monitors.remove(index);
        match self.monitors.first_mut() {
            Some(target) => target.workspaces.absorb(removed.workspaces),
            None => self.detached = Some(removed.workspaces),
        }
        self.reconcile();
    }

    /// Give every monitor its home workspace under the right number, and bring workspaces
    /// back to their owning monitor (after hotplug, or once a connector name became known).
    /// Returns whether anything moved.
    pub fn reconcile(&mut self) -> bool {
        if self.monitors.is_empty() {
            return false;
        }
        let mut changed = false;
        let homes: Vec<_> = (0..self.monitors.len()).map(|i| self.home_number(i)).collect();
        // Extras never keep a number that a monitor's home workspace needs.
        for monitor in 0..self.monitors.len() {
            let colliding: Vec<_> = self.monitors[monitor]
                .workspaces
                .entries
                .iter()
                .filter(|entry| entry.kind == Kind::Extra && homes.contains(&entry.number))
                .map(|entry| entry.number)
                .collect();
            for old in colliding {
                if let Some(new) = self.free_number() {
                    self.monitors[monitor].workspaces.renumber(old, new);
                    changed = true;
                }
            }
        }
        for (monitor, home) in homes.iter().enumerate() {
            let workspaces = &mut self.monitors[monitor].workspaces;
            if workspaces.home != *home {
                workspaces.renumber(workspaces.home, *home);
                changed = true;
            }
            workspaces.ensure(*home, Kind::Home);
        }
        let gaming = self.gaming_monitor().unwrap();
        for source in 0..self.monitors.len() {
            let misplaced: Vec<_> = self.monitors[source]
                .workspaces
                .entries
                .iter()
                .filter_map(|entry| {
                    let target = match entry.kind {
                        Kind::Home => homes.iter().position(|home| *home == entry.number)?,
                        Kind::Gaming => gaming,
                        Kind::Extra => return None,
                    };
                    (target != source).then_some((entry.number, target))
                })
                .collect();
            for (number, target) in misplaced {
                if let Some(moved) = self.monitors[source].workspaces.take(number) {
                    self.monitors[target].workspaces.insert(moved);
                    changed = true;
                }
            }
        }
        changed
    }

    /// Remove extras that were left empty and the gaming workspace once its last window is
    /// gone; a monitor returns to the workspace it came from. Returns the monitors whose
    /// shown workspace changed.
    pub fn prune(&mut self) -> Vec<usize> {
        let homes: Vec<_> = (0..self.monitors.len()).map(|i| self.home_number(i)).collect();
        let mut switched = Vec::new();
        for (index, monitor) in self.monitors.iter_mut().enumerate() {
            // A missing monitor's empty home workspace is no longer needed.
            for entry in &mut monitor.workspaces.entries {
                if entry.kind == Kind::Home && entry.windows.is_empty() && !homes.contains(&entry.number) {
                    entry.kind = Kind::Extra;
                }
            }
            if monitor.workspaces.prune() {
                switched.push(index);
            }
        }
        switched
    }

    /// Monitor and workspace number holding `window`.
    pub fn locate(&self, window: &T) -> Option<(usize, usize)> {
        self.monitors
            .iter()
            .enumerate()
            .find_map(|(index, m)| m.workspaces.location(window).map(|number| (index, number)))
    }

    pub fn remove_window(&mut self, window: &T) {
        for monitor in &mut self.monitors {
            monitor.workspaces.remove(window);
        }
        if let Some(detached) = &mut self.detached {
            detached.remove(window);
        }
    }

    /// Move `window` to workspace `number` (on whichever monitor owns it). Returns the
    /// destination monitor; `None` if no such workspace exists.
    pub fn move_window(&mut self, window: &T, number: usize) -> Option<usize> {
        let target = self.owner(number)?;
        self.remove_window(window);
        self.monitors[target].workspaces.add_to(number, window.clone());
        Some(target)
    }

    pub fn monitor_at(&self, x: i32, y: i32) -> Option<usize> {
        self.monitors.iter().position(|m| {
            x >= m.area.x && x < m.area.x + m.area.width && y >= m.area.y && y < m.area.y + m.area.height
        })
    }

    /// Keep a pointer position on some monitor: positions already on one are unchanged,
    /// others snap to the nearest point of the nearest monitor.
    pub fn clamp_to_monitors(&self, (x, y): (f64, f64)) -> (f64, f64) {
        let mut best: Option<(f64, (f64, f64))> = None;
        for monitor in &self.monitors {
            let a = monitor.area;
            // The last valid pixel is `right - 1`; stay just inside.
            let cx = x.clamp(f64::from(a.x), f64::from(a.x + a.width) - 1.0);
            let cy = y.clamp(f64::from(a.y), f64::from(a.y + a.height) - 1.0);
            let distance = (cx - x).powi(2) + (cy - y).powi(2);
            if best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, (cx, cy)));
            }
        }
        best.map_or((x, y), |(_, point)| point)
    }

    /// The monitor nearest to `source` in `direction`, by distance between centers.
    pub fn adjacent(&self, source: usize, direction: Direction) -> Option<usize> {
        let center = |r: Rect| (r.x + r.width / 2, r.y + r.height / 2);
        let (sx, sy) = center(self.monitors.get(source)?.area);
        self.monitors
            .iter()
            .enumerate()
            .filter_map(|(index, monitor)| {
                if index == source {
                    return None;
                }
                let (cx, cy) = center(monitor.area);
                let (dx, dy) = (cx - sx, cy - sy);
                let (forward, sideways) = match direction {
                    Direction::Left if dx < 0 => (-dx, dy.abs()),
                    Direction::Right if dx > 0 => (dx, dy.abs()),
                    Direction::Up if dy < 0 => (-dy, dx.abs()),
                    Direction::Down if dy > 0 => (dy, dx.abs()),
                    _ => return None,
                };
                Some((i64::from(sideways).pow(2) + i64::from(forward).pow(2), index))
            })
            .min_by_key(|candidate| candidate.0)
            .map(|candidate| candidate.1)
    }

    /// The workspace `direction` steps away on this monitor, wrapping around (gaming included).
    pub fn relative_workspace(&self, monitor: usize, direction: isize) -> Option<usize> {
        let workspaces = &self.monitors[monitor].workspaces;
        let numbers: Vec<_> = workspaces.numbers().collect();
        let position = numbers.iter().position(|number| *number == workspaces.active)?;
        Some(numbers[(position as isize + direction).rem_euclid(numbers.len() as isize) as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect { x, y, width, height }
    }

    fn desk(listed: &[&str]) -> Desk<u32> {
        Desk::new(listed.iter().map(|s| s.to_string()).collect(), None)
    }

    fn numbers(desk: &Desk<u32>, monitor: usize) -> Vec<usize> {
        desk.monitors[monitor].workspaces.numbers().collect()
    }

    #[test]
    fn unlisted_monitors_get_consecutive_home_numbers() {
        let mut desk = desk(&[]);
        desk.add_monitor(Some("A".into()), area(0, 0, 100, 100));
        desk.add_monitor(Some("B".into()), area(100, 0, 100, 100));
        assert_eq!((desk.home_number(0), desk.home_number(1)), (1, 2));
        assert_eq!(numbers(&desk, 1), [2]);
    }

    #[test]
    fn listed_names_decide_the_home_number_regardless_of_plug_order() {
        let mut desk = desk(&["DP-3", "HDMI-A-1"]);
        desk.add_monitor(Some("HDMI-A-1".into()), area(0, 0, 100, 100));
        desk.add_monitor(Some("eDP-1".into()), area(100, 0, 100, 100));
        desk.add_monitor(Some("DP-3".into()), area(200, 0, 100, 100));
        assert_eq!((desk.home_number(0), desk.home_number(1), desk.home_number(2)), (2, 3, 1));
        assert_eq!(numbers(&desk, 2), [1]);
        assert_eq!(desk.owner(3), Some(1));
    }

    #[test]
    fn extras_use_free_numbers_and_never_collide_with_homes() {
        let mut desk = desk(&["A", "B"]);
        desk.add_monitor(Some("A".into()), area(0, 0, 100, 100));
        assert_eq!(desk.free_number(), Some(3), "2 is reserved for B even while unplugged");
        let number = desk.create(0).unwrap();
        assert_eq!(number, 3);
        assert_eq!(desk.create(0), Some(3), "an empty extra is reused");
        desk.monitors[0].workspaces.add_to(3, 7);
        assert_eq!(desk.create(0), Some(4));
    }

    #[test]
    fn unplugging_moves_workspaces_and_replugging_brings_them_back() {
        let mut desk = desk(&["A", "B"]);
        desk.add_monitor(Some("A".into()), area(0, 0, 100, 100));
        desk.add_monitor(Some("B".into()), area(100, 0, 100, 100));
        desk.monitors[1].workspaces.add(5);
        desk.remove_monitor(1);
        assert_eq!(numbers(&desk, 0), [1, 2]);
        assert_eq!(desk.locate(&5), Some((0, 2)));
        desk.add_monitor(Some("B".into()), area(100, 0, 100, 100));
        assert_eq!(numbers(&desk, 0), [1]);
        assert_eq!(numbers(&desk, 1), [2]);
        assert_eq!(desk.locate(&5), Some((1, 2)));
    }

    #[test]
    fn last_monitor_gone_keeps_workspaces_until_one_returns() {
        let mut desk = desk(&[]);
        desk.add_monitor(Some("A".into()), area(0, 0, 100, 100));
        desk.monitors[0].workspaces.add(1);
        desk.remove_monitor(0);
        assert!(desk.monitors.is_empty());
        desk.add_monitor(Some("B".into()), area(0, 0, 100, 100));
        assert_eq!(desk.locate(&1), Some((0, 1)));
    }

    #[test]
    fn late_connector_name_moves_the_home_workspace() {
        let mut desk = desk(&["DP-1", "DP-2"]);
        desk.add_monitor(None, area(0, 0, 100, 100));
        desk.add_monitor(None, area(100, 0, 100, 100));
        assert_eq!((desk.home_number(0), desk.home_number(1)), (3, 4));
        desk.monitors[0].name = Some("DP-2".into());
        desk.monitors[1].name = Some("DP-1".into());
        desk.monitors[0].workspaces.add(9);
        assert!(desk.reconcile());
        assert_eq!((desk.home_number(0), desk.home_number(1)), (2, 1));
        assert_eq!(desk.locate(&9), Some((0, 2)));
    }

    #[test]
    fn gaming_workspace_lives_on_the_gaming_monitor() {
        let mut desk = Desk::<u32>::new(vec![], Some("B".into()));
        desk.add_monitor(Some("A".into()), area(0, 0, 100, 100));
        desk.add_monitor(Some("B".into()), area(100, 0, 100, 100));
        assert_eq!(desk.ensure_gaming(), Some(1));
        assert_eq!(desk.owner(GAMING), Some(1));
        assert_eq!(desk.move_window(&4, GAMING), Some(1));
        assert_eq!(desk.locate(&4), Some((1, GAMING)));
        desk.remove_window(&4);
        desk.monitors[1].workspaces.select(GAMING);
        assert_eq!(desk.prune(), vec![1], "the monitor left the emptied gaming workspace");
        assert_eq!(desk.monitors[1].workspaces.active, 2);
    }

    #[test]
    fn moving_across_monitors_and_within_one() {
        let mut desk = desk(&[]);
        desk.add_monitor(Some("A".into()), area(0, 0, 100, 100));
        desk.add_monitor(Some("B".into()), area(100, 0, 100, 100));
        desk.monitors[0].workspaces.add(1);
        assert_eq!(desk.move_window(&1, 2), Some(1));
        assert_eq!(desk.locate(&1), Some((1, 2)));
        assert_eq!(desk.move_window(&1, 8), None);
        assert_eq!(desk.locate(&1), Some((1, 2)), "a failed move leaves the window alone");
    }

    #[test]
    fn adjacency_follows_the_users_layout() {
        // HDMI-A-1 top left, DP-3 below it, DP-1 to the right (the author's setup).
        let mut desk = desk(&[]);
        desk.add_monitor(Some("HDMI-A-1".into()), area(0, 0, 2560, 1080));
        desk.add_monitor(Some("DP-3".into()), area(0, 1080, 2560, 1440));
        desk.add_monitor(Some("DP-1".into()), area(2560, 0, 1440, 2560));
        assert_eq!(desk.adjacent(0, Direction::Down), Some(1));
        assert_eq!(desk.adjacent(1, Direction::Up), Some(0));
        assert_eq!(desk.adjacent(0, Direction::Right), Some(2));
        assert_eq!(desk.adjacent(1, Direction::Right), Some(2));
        assert_eq!(desk.adjacent(2, Direction::Left), Some(1), "DP-3's center is closer than HDMI-A-1's");
        assert_eq!(desk.adjacent(0, Direction::Left), None);
        assert_eq!(desk.monitor_at(2600, 10), Some(2));
        assert_eq!(desk.monitor_at(-1, 0), None);
    }

    #[test]
    fn pointer_stays_on_the_monitors() {
        let mut desk = desk(&[]);
        desk.add_monitor(Some("A".into()), area(0, 0, 100, 100));
        desk.add_monitor(Some("B".into()), area(100, 50, 100, 100));
        assert_eq!(desk.clamp_to_monitors((50.5, 20.0)), (50.5, 20.0));
        assert_eq!(desk.clamp_to_monitors((-30.0, 20.0)), (0.0, 20.0));
        // Above B but right of A's top edge: sliding along A is not allowed past its corner.
        assert_eq!(desk.clamp_to_monitors((150.0, 10.0)), (150.0, 50.0));
        assert_eq!(desk.clamp_to_monitors((500.0, 500.0)), (199.0, 149.0));
        assert_eq!(Desk::<u32>::new(vec![], None).clamp_to_monitors((3.0, 4.0)), (3.0, 4.0));
    }

    #[test]
    fn relative_workspace_wraps() {
        let mut desk = desk(&[]);
        desk.add_monitor(Some("A".into()), area(0, 0, 100, 100));
        desk.create(0);
        assert_eq!(desk.relative_workspace(0, 1), Some(2));
        assert_eq!(desk.relative_workspace(0, -1), Some(2));
    }
}
