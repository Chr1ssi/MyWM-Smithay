/// Number reserved for the dynamic gaming workspace. Regular workspaces are 1..=9.
pub const GAMING: usize = 0;
/// Highest number a regular workspace can have (keys 1 to 9 address them).
pub const MAX_NUMBER: usize = 9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The fixed workspace of a monitor; never removed.
    Home,
    /// Dynamically created; removed once it is empty and no longer shown.
    Extra,
    /// Exists only while a game runs; removed as soon as it is empty.
    Gaming,
}

/// A workspace owns its window order, remembered focus and horizontal viewport.
#[derive(Debug)]
pub struct Workspace<T> {
    pub number: usize,
    pub kind: Kind,
    pub windows: Vec<T>,
    pub focused: Option<T>,
    pub scroll: i32,
}

impl<T> Default for Workspace<T> {
    fn default() -> Self {
        Self::new(1, Kind::Home)
    }
}

impl<T> Workspace<T> {
    pub fn new(number: usize, kind: Kind) -> Self {
        Self {
            number,
            kind,
            windows: Vec::new(),
            focused: None,
            scroll: 0,
        }
    }
}

impl<T: Clone + Eq> Workspace<T> {
    pub fn remove(&mut self, window: &T) {
        if let Some(index) = self.windows.iter().position(|id| id == window) {
            self.windows.remove(index);
            if self.focused.as_ref() == Some(window) {
                self.focused = self
                    .windows
                    .get(index)
                    .or_else(|| self.windows.last())
                    .cloned();
            }
        }
    }
}

fn order(number: usize) -> usize {
    if number == GAMING { usize::MAX } else { number }
}

/// The workspaces shown on one monitor, kept in display order: the home
/// workspace(s) first, then extras by number, the gaming workspace last.
/// `active` always names an existing entry.
#[derive(Debug)]
pub struct Workspaces<T> {
    pub active: usize,
    /// Workspace number the monitor was last active on before `active`.
    pub previous: Option<usize>,
    pub home: usize,
    pub entries: Vec<Workspace<T>>,
}

impl<T: Clone + Eq> Workspaces<T> {
    pub fn new(home: usize) -> Self {
        Self {
            active: home,
            previous: None,
            home,
            entries: vec![Workspace::new(home, Kind::Home)],
        }
    }

    pub fn current(&self) -> &Workspace<T> {
        self.get(self.active).expect("active workspace exists")
    }

    pub fn current_mut(&mut self) -> &mut Workspace<T> {
        let active = self.active;
        self.get_mut(active).expect("active workspace exists")
    }

    pub fn get(&self, number: usize) -> Option<&Workspace<T>> {
        self.entries.iter().find(|entry| entry.number == number)
    }

    pub fn get_mut(&mut self, number: usize) -> Option<&mut Workspace<T>> {
        self.entries.iter_mut().find(|entry| entry.number == number)
    }

    pub fn contains(&self, number: usize) -> bool {
        self.get(number).is_some()
    }

    pub fn numbers(&self) -> impl Iterator<Item = usize> + '_ {
        self.entries.iter().map(|entry| entry.number)
    }

    /// Return the workspace with this number, creating it if necessary.
    pub fn ensure(&mut self, number: usize, kind: Kind) -> &mut Workspace<T> {
        if !self.contains(number) {
            self.entries.push(Workspace::new(number, kind));
            self.entries.sort_by_key(|entry| order(entry.number));
        }
        self.get_mut(number).unwrap()
    }

    /// Lowest regular workspace number that does not exist yet.
    pub fn free_number(&self) -> Option<usize> {
        (1..=MAX_NUMBER).find(|number| !self.contains(*number))
    }

    /// The next (`1`) or previous (`-1`) regular workspace, wrapping around.
    pub fn relative(&self, direction: isize) -> Option<usize> {
        let numbers: Vec<_> = self.numbers().filter(|number| *number != GAMING).collect();
        let position = numbers.iter().position(|number| *number == self.active)?;
        let next = (position as isize + direction).rem_euclid(numbers.len() as isize) as usize;
        Some(numbers[next])
    }

    /// Select the next (`1`) or previous (`-1`) regular workspace, wrapping around.
    pub fn cycle(&mut self, direction: isize) {
        if let Some(number) = self.relative(direction) {
            self.select(number);
        }
    }

    pub fn select(&mut self, number: usize) {
        if number != self.active && self.contains(number) {
            self.previous = Some(self.active);
            self.active = number;
        }
    }

    pub fn add(&mut self, window: T) {
        self.add_to(self.active, window);
    }

    pub fn add_to(&mut self, number: usize, window: T) {
        let kind = if number == GAMING {
            Kind::Gaming
        } else {
            Kind::Extra
        };
        let workspace = self.ensure(number, kind);
        workspace.focused = Some(window.clone());
        workspace.windows.push(window);
    }

    pub fn location(&self, window: &T) -> Option<usize> {
        self.entries
            .iter()
            .find(|workspace| workspace.windows.contains(window))
            .map(|workspace| workspace.number)
    }

    /// Change a workspace's number, merging into an existing one of that number.
    pub fn renumber(&mut self, old: usize, new: usize) {
        if old == new {
            return;
        }
        if let Some(index) = self.entries.iter().position(|entry| entry.number == old) {
            let moved = self.entries.remove(index);
            self.insert(Workspace {
                number: new,
                ..moved
            });
        }
        if self.active == old {
            self.active = new;
        }
        if self.previous == Some(old) {
            self.previous = Some(new);
        }
        if self.home == old {
            self.home = new;
        }
    }

    /// Drop workspaces that ended their life. Extras go once empty and not shown;
    /// the gaming workspace goes as soon as it is empty, returning to the
    /// previous workspace. Returns whether the displayed workspace changed.
    pub fn prune(&mut self) -> bool {
        let active = self.active;
        // While a game is shown, the workspace we return to must survive, even
        // if it is an empty extra.
        let returning = self
            .previous
            .filter(|_| self.get(active).is_some_and(|entry| entry.kind == Kind::Gaming));
        self.entries.retain(|entry| {
            !entry.windows.is_empty()
                || match entry.kind {
                    Kind::Home => true,
                    Kind::Extra => entry.number == active || Some(entry.number) == returning,
                    Kind::Gaming => false,
                }
        });
        if self.contains(self.active) {
            return false;
        }
        self.active = self
            .previous
            .filter(|number| self.contains(*number))
            .or_else(|| self.contains(self.home).then_some(self.home))
            .unwrap_or_else(|| self.entries[0].number);
        true
    }

    pub fn focus(&mut self, window: &T) -> bool {
        if self.current().windows.contains(window) {
            self.current_mut().focused = Some(window.clone());
            true
        } else {
            false
        }
    }

    pub fn navigate(&mut self, direction: isize, reorder: bool) {
        self.navigate_matching(direction, reorder, |_| true);
    }

    pub fn navigate_matching(
        &mut self,
        direction: isize,
        reorder: bool,
        mut include: impl FnMut(&T) -> bool,
    ) {
        let workspace = self.current_mut();
        let indices: Vec<_> = workspace
            .windows
            .iter()
            .enumerate()
            .filter_map(|(i, id)| include(id).then_some(i))
            .collect();
        let Some(slot) = indices
            .iter()
            .position(|i| Some(&workspace.windows[*i]) == workspace.focused.as_ref())
        else {
            return;
        };
        let Some(next) = slot
            .checked_add_signed(direction)
            .and_then(|i| indices.get(i))
            .copied()
        else {
            return;
        };
        if reorder {
            workspace.windows.swap(indices[slot], next);
        } else {
            workspace.focused = Some(workspace.windows[next].clone());
        }
    }

    pub fn can_navigate_matching(
        &self,
        direction: isize,
        mut include: impl FnMut(&T) -> bool,
    ) -> bool {
        let workspace = self.current();
        let indices: Vec<_> = workspace
            .windows
            .iter()
            .enumerate()
            .filter_map(|(index, id)| include(id).then_some(index))
            .collect();
        indices
            .iter()
            .position(|index| Some(&workspace.windows[*index]) == workspace.focused.as_ref())
            .and_then(|slot| slot.checked_add_signed(direction))
            .is_some_and(|slot| slot < indices.len())
    }

    pub fn remove(&mut self, window: &T) {
        for workspace in &mut self.entries {
            if workspace.windows.contains(window) {
                workspace.remove(window);
                if workspace.windows.is_empty() {
                    workspace.scroll = 0;
                }
                return;
            }
        }
    }

    #[cfg(test)]
    pub fn move_focused_to(&mut self, target: usize) {
        if target == self.active || !self.contains(target) {
            return;
        }
        let Some(window) = self.current().focused.clone() else {
            return;
        };
        self.remove(&window);
        self.add_to(target, window);
    }

    /// Add a workspace, merging into an existing one of the same number.
    pub fn insert(&mut self, source: Workspace<T>) {
        match self.get_mut(source.number) {
            Some(target) => {
                if source.kind == Kind::Home {
                    target.kind = Kind::Home;
                }
                target.windows.extend(source.windows);
                if target.focused.is_none() {
                    target.focused = source.focused;
                }
            }
            None => {
                self.entries.push(source);
                self.entries.sort_by_key(|entry| order(entry.number));
            }
        }
    }

    /// Remove a workspace, falling back to another one if it was shown. The
    /// home workspace must exist unless it is the one being taken.
    pub fn take(&mut self, number: usize) -> Option<Workspace<T>> {
        let index = self.entries.iter().position(|entry| entry.number == number)?;
        if self.entries.len() == 1 {
            return None;
        }
        let taken = self.entries.remove(index);
        if self.active == number {
            self.active = self
                .previous
                .filter(|previous| self.contains(*previous))
                .or_else(|| self.contains(self.home).then_some(self.home))
                .unwrap_or_else(|| self.entries[0].number);
        }
        Some(taken)
    }

    /// Keep a removed monitor's workspaces (and their numbers) on another one.
    pub fn absorb(&mut self, other: Self) {
        for source in other.entries {
            self.insert(source);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_extra(windows: &[i32]) -> Workspaces<i32> {
        let mut desktop = Workspaces::new(1);
        desktop.ensure(4, Kind::Extra);
        for window in windows {
            desktop.add(*window);
        }
        desktop
    }

    #[test]
    fn switching_restores_focus_order_and_scroll() {
        let mut desktop = with_extra(&[1, 2, 3]);
        desktop.current_mut().scroll = 960;
        desktop.select(4);
        assert!(desktop.current().windows.is_empty());
        assert_eq!(desktop.current().focused, None);
        desktop.add(4);
        desktop.select(1);
        assert_eq!(desktop.current().windows, [1, 2, 3]);
        assert_eq!(desktop.current().focused, Some(3));
        assert_eq!(desktop.current().scroll, 960);
        assert_eq!(desktop.previous, Some(4));
        desktop.select(9);
        assert_eq!(desktop.active, 1);
    }

    #[test]
    fn moving_does_not_follow_and_never_duplicates_a_window() {
        let mut desktop = with_extra(&[1, 2]);
        desktop.move_focused_to(4);
        assert_eq!(desktop.active, 1);
        assert_eq!(desktop.current().windows, [1]);
        assert_eq!(desktop.current().focused, Some(1));
        desktop.move_focused_to(4);
        assert_eq!(desktop.current().focused, None);
        desktop.select(4);
        assert_eq!(desktop.current().windows, [2, 1]);
        assert_eq!(desktop.current().focused, Some(1));
        desktop.move_focused_to(4);
        assert_eq!(desktop.current().windows, [2, 1]);
    }

    #[test]
    fn free_numbers_and_cycling_skip_gaming() {
        let mut desktop = Workspaces::new(1);
        assert_eq!(desktop.free_number(), Some(2));
        desktop.ensure(2, Kind::Extra);
        desktop.ensure(4, Kind::Extra);
        desktop.add_to(GAMING, 9);
        assert_eq!(desktop.free_number(), Some(3));
        desktop.cycle(1);
        assert_eq!(desktop.active, 2);
        desktop.cycle(1);
        desktop.cycle(1);
        assert_eq!(desktop.active, 1, "wraps past 4 without entering gaming");
        desktop.cycle(-1);
        assert_eq!(desktop.active, 4);
    }

    #[test]
    fn navigation_and_removal_stay_inside_workspace() {
        let mut desktop = with_extra(&[1, 2]);
        desktop.select(4);
        desktop.add(3);
        assert!(!desktop.focus(&1));
        desktop.remove(&2);
        assert_eq!(desktop.current().focused, Some(3));
        desktop.select(1);
        assert_eq!(desktop.current().focused, Some(1));
        desktop.add(4);
        desktop.navigate(-1, true);
        assert_eq!(desktop.current().windows, [4, 1]);
        assert_eq!(desktop.current().focused, Some(4));
        desktop.navigate(-1, false);
        assert_eq!(desktop.current().focused, Some(4));
        desktop.navigate(1, false);
        assert_eq!(desktop.current().focused, Some(1));
    }

    #[test]
    fn reordering_columns_skips_floating_windows() {
        let mut desktop = Workspaces::new(1);
        desktop.add(1);
        desktop.add(2); // floating, retains its position in keyboard focus order
        desktop.add(3);
        desktop.navigate_matching(-1, true, |id| *id != 2);
        assert_eq!(desktop.current().windows, [3, 2, 1]);
        assert_eq!(desktop.current().focused, Some(3));
        assert!(!desktop.can_navigate_matching(-1, |id| *id != 2));
        assert!(desktop.can_navigate_matching(1, |id| *id != 2));
        desktop.navigate(1, false);
        assert_eq!(desktop.current().focused, Some(2));
    }

    #[test]
    fn unplug_preserves_workspace_numbers() {
        let mut left = Workspaces::new(1);
        let mut right = Workspaces::new(2);
        left.add(1);
        right.add(3);
        right.ensure(5, Kind::Extra);
        right.select(5);
        right.add(4);
        left.absorb(right);
        assert_eq!(left.numbers().collect::<Vec<_>>(), [1, 2, 5]);
        assert_eq!(left.current().windows, [1]);
        left.select(2);
        assert_eq!(left.current().windows, [3]);
        left.select(5);
        assert_eq!(left.current().windows, [4]);
    }

    #[test]
    fn extras_disappear_once_empty_and_not_shown() {
        let mut desktop = with_extra(&[1]);
        desktop.select(4);
        assert!(!desktop.prune());
        assert!(desktop.contains(4), "the shown extra stays while empty");
        desktop.select(1);
        assert!(!desktop.prune());
        assert!(!desktop.contains(4));
        assert!(desktop.contains(1), "home is permanent even when empty");
        desktop.remove(&1);
        desktop.prune();
        assert!(desktop.contains(1));
    }

    #[test]
    fn gaming_workspace_closes_with_its_last_window_and_returns() {
        let mut desktop = with_extra(&[1]);
        desktop.select(4);
        desktop.add_to(GAMING, 7);
        desktop.select(GAMING);
        assert_eq!(desktop.numbers().last(), Some(GAMING));
        assert!(!desktop.prune());
        desktop.remove(&7);
        assert!(desktop.prune());
        assert_eq!(desktop.active, 4);
        assert!(!desktop.contains(GAMING));
        desktop.select(1);
        assert!(!desktop.prune());
        assert!(!desktop.contains(4));
        desktop.add_to(GAMING, 8);
        desktop.select(GAMING);
        desktop.remove(&8);
        assert!(desktop.prune());
        assert_eq!(desktop.active, 1);
        assert!(!desktop.contains(GAMING));
    }

    #[test]
    fn renumbering_moves_the_home_workspace_and_merges_duplicates() {
        let mut desktop = Workspaces::new(4);
        desktop.add(1);
        desktop.ensure(1, Kind::Extra);
        desktop.add_to(1, 2);
        desktop.renumber(4, 1);
        assert_eq!((desktop.active, desktop.home), (1, 1));
        assert_eq!(desktop.current().kind, Kind::Home);
        assert_eq!(desktop.numbers().collect::<Vec<_>>(), [1]);
        assert_eq!(desktop.current().windows, [2, 1]);
    }
}
