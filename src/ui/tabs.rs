#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct WorkspaceTabId(u64);

#[derive(Debug)]
pub(super) struct WorkspaceTabsModel {
    tabs: Vec<WorkspaceTabId>,
    active_index: usize,
    next_id: u64,
}

impl WorkspaceTabsModel {
    pub(super) fn new() -> Self {
        Self {
            tabs: vec![WorkspaceTabId(1)],
            active_index: 0,
            next_id: 1,
        }
    }

    pub(super) fn tabs(&self) -> &[WorkspaceTabId] {
        &self.tabs
    }

    pub(super) fn active(&self) -> WorkspaceTabId {
        self.tabs[self.active_index]
    }

    pub(super) fn add(&mut self) -> WorkspaceTabId {
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("workspace tab id exhausted");
        let id = WorkspaceTabId(self.next_id);
        self.tabs.push(id);
        self.active_index = self.tabs.len() - 1;
        id
    }

    pub(super) fn activate(&mut self, id: WorkspaceTabId) -> bool {
        let Some(index) = self.tabs.iter().position(|candidate| *candidate == id) else {
            return false;
        };
        let changed = self.active_index != index;
        self.active_index = index;
        changed
    }

    pub(super) fn close(&mut self, id: WorkspaceTabId) -> bool {
        if self.tabs.len() == 1 {
            return false;
        }
        let Some(index) = self.tabs.iter().position(|candidate| *candidate == id) else {
            return false;
        };
        self.tabs.remove(index);
        if index < self.active_index || self.active_index == self.tabs.len() {
            self.active_index -= 1;
        }
        true
    }

    pub(super) fn next(&mut self) -> WorkspaceTabId {
        self.active_index = (self.active_index + 1) % self.tabs.len();
        self.active()
    }

    pub(super) fn previous(&mut self) -> WorkspaceTabId {
        self.active_index = (self.active_index + self.tabs.len() - 1) % self.tabs.len();
        self.active()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_navigation_wraps_and_close_keeps_a_valid_active_tab() {
        let mut tabs = WorkspaceTabsModel::new();
        let first = tabs.active();
        let second = tabs.add();
        let third = tabs.add();

        assert_eq!(tabs.next(), first);
        assert_eq!(tabs.previous(), third);
        assert!(tabs.close(third));
        assert_eq!(tabs.active(), second);
        assert!(tabs.close(first));
        assert_eq!(tabs.active(), second);
        assert!(!tabs.close(second));
    }

    #[test]
    fn activating_an_unknown_tab_does_not_change_selection() {
        let mut tabs = WorkspaceTabsModel::new();
        let active = tabs.active();

        assert!(!tabs.activate(WorkspaceTabId(999)));
        assert_eq!(tabs.active(), active);
    }
}
