use crate::catalog::{AccentColor, Catalog};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VisibleRow {
    Group(usize),
    Host(usize),
}

pub(super) struct Group {
    pub name: String,
    pub color: Option<AccentColor>,
    pub total: usize,
    pub matches: Vec<usize>,
}

/// A projection of the immutable catalog, rebuilt only on filter, expansion or
/// reload. Navigation and rendering borrow these rows without repeating fuzzy
/// matching or sorting, regardless of how many panes are visible.
pub(super) struct HostList {
    pub rows: Vec<VisibleRow>,
    pub groups: Vec<Group>,
    pub shown_hosts: usize,
    host_groups: Vec<Option<usize>>,
    ungrouped: Vec<usize>,
    expanded: HashSet<String>,
    search_collapsed: HashSet<usize>,
    searching: bool,
}

impl HostList {
    pub fn new(catalog: &Catalog) -> Self {
        let mut groups: Vec<Group> = Vec::new();
        let mut group_indices = HashMap::new();
        let host_groups = catalog
            .hosts()
            .iter()
            .map(|host| {
                host.metadata.group.as_ref().map(|name| {
                    let index = *group_indices.entry(name.as_str()).or_insert_with(|| {
                        groups.push(Group {
                            name: name.clone(),
                            color: host.metadata.color,
                            total: 0,
                            matches: Vec::new(),
                        });
                        groups.len() - 1
                    });
                    groups[index].total += 1;
                    index
                })
            })
            .collect();
        let mut list = Self {
            rows: Vec::new(),
            groups,
            shown_hosts: 0,
            host_groups,
            ungrouped: Vec::new(),
            expanded: HashSet::new(),
            search_collapsed: HashSet::new(),
            searching: false,
        };
        list.filter(catalog, "");
        list
    }

    pub fn filter(&mut self, catalog: &Catalog, query: &str) {
        self.searching = !query.trim().is_empty();
        self.search_collapsed.clear();
        self.ungrouped.clear();
        for group in &mut self.groups {
            group.matches.clear();
        }
        for index in catalog.filtered_indices(query) {
            match self.host_groups[index] {
                Some(group) => self.groups[group].matches.push(index),
                None => self.ungrouped.push(index),
            }
        }
        self.rebuild_rows();
    }

    pub fn is_expanded(&self, group: usize) -> bool {
        if self.searching {
            !self.search_collapsed.contains(&group)
        } else {
            self.expanded.contains(&self.groups[group].name)
        }
    }

    pub fn toggle(&mut self, group: usize) {
        // Search expansion is temporary. Clearing the query restores exactly
        // the tree the user was browsing before searching.
        if self.searching {
            if !self.search_collapsed.remove(&group) {
                self.search_collapsed.insert(group);
            }
        } else {
            let name = &self.groups[group].name;
            if !self.expanded.remove(name) {
                self.expanded.insert(name.clone());
            }
        }
        self.rebuild_rows();
    }

    pub fn restore_expansion(&mut self, previous: &Self) {
        self.expanded = self
            .groups
            .iter()
            .filter(|group| previous.expanded.contains(&group.name))
            .map(|group| group.name.clone())
            .collect();
        self.rebuild_rows();
    }

    fn rebuild_rows(&mut self) {
        self.rows.clear();
        self.shown_hosts = self.ungrouped.len();
        for (index, group) in self.groups.iter().enumerate() {
            if group.matches.is_empty() {
                continue;
            }
            self.rows.push(VisibleRow::Group(index));
            if self.is_expanded(index) {
                self.rows
                    .extend(group.matches.iter().copied().map(VisibleRow::Host));
                self.shown_hosts += group.matches.len();
            }
        }
        self.rows
            .extend(self.ungrouped.iter().copied().map(VisibleRow::Host));
    }
}
