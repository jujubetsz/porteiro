use crate::catalog::{AccentColor, Catalog};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VisibleRow {
    Group(usize),
    Host(usize),
}

pub(super) struct Group {
    pub name: String,
    pub path: Vec<String>,
    pub color: Option<AccentColor>,
    pub total: usize,
    pub matched: usize,
    parent: Option<usize>,
    children: Vec<usize>,
    hosts: Vec<usize>,
}

/// A projection of the immutable catalog, rebuilt only on filter, expansion or
/// reload. Navigation and rendering borrow these rows without repeating fuzzy
/// matching or sorting, regardless of how many panes are visible.
pub(super) struct HostList {
    pub rows: Vec<VisibleRow>,
    pub groups: Vec<Group>,
    pub shown_hosts: usize,
    prefixes: Vec<String>,
    roots: Vec<usize>,
    host_groups: Vec<Option<usize>>,
    ungrouped: Vec<usize>,
    expanded: HashSet<Vec<String>>,
    search_collapsed: HashSet<usize>,
    searching: bool,
}

impl HostList {
    pub fn new(catalog: &Catalog) -> Self {
        let mut groups: Vec<Group> = Vec::new();
        let mut group_indices = HashMap::new();
        let mut roots = Vec::new();
        let host_groups = catalog
            .hosts()
            .iter()
            .map(|host| {
                let mut parent: Option<usize> = None;
                let mut path = Vec::new();
                for name in &host.metadata.group {
                    path.push(name.clone());
                    let index = *group_indices.entry(path.clone()).or_insert_with(|| {
                        let index = groups.len();
                        if let Some(parent) = parent {
                            groups[parent].children.push(index);
                        } else {
                            roots.push(index);
                        }
                        groups.push(Group {
                            name: name.clone(),
                            path: path.clone(),
                            color: host.metadata.color,
                            total: 0,
                            matched: 0,
                            parent,
                            children: Vec::new(),
                            hosts: Vec::new(),
                        });
                        index
                    });
                    groups[index].total += 1;
                    parent = Some(index);
                }
                parent
            })
            .collect();
        let mut list = Self {
            rows: Vec::new(),
            groups,
            shown_hosts: 0,
            prefixes: Vec::new(),
            roots,
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
            group.hosts.clear();
            group.matched = 0;
        }
        for index in catalog.filtered_indices(query) {
            match self.host_groups[index] {
                Some(group) => {
                    self.groups[group].hosts.push(index);
                    let mut ancestor = Some(group);
                    while let Some(group) = ancestor {
                        self.groups[group].matched += 1;
                        ancestor = self.groups[group].parent;
                    }
                }
                None => self.ungrouped.push(index),
            }
        }
        self.rebuild_rows();
    }

    pub fn is_expanded(&self, group: usize) -> bool {
        if self.searching {
            !self.search_collapsed.contains(&group)
        } else {
            self.expanded.contains(&self.groups[group].path)
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
            let path = &self.groups[group].path;
            if !self.expanded.remove(path) {
                self.expanded.insert(path.clone());
            }
        }
        self.rebuild_rows();
    }

    pub fn restore_expansion(&mut self, previous: &Self) {
        self.expanded = self
            .groups
            .iter()
            .filter(|group| previous.expanded.contains(&group.path))
            .map(|group| group.path.clone())
            .collect();
        self.rebuild_rows();
    }

    fn rebuild_rows(&mut self) {
        self.rows.clear();
        self.prefixes.clear();
        self.shown_hosts = 0;
        let mut pending: Vec<_> = self
            .roots
            .iter()
            .rev()
            .filter(|&&index| self.groups[index].matched > 0)
            .map(|&index| (VisibleRow::Group(index), Vec::<bool>::new()))
            .collect();
        // Iterative traversal also handles deeply nested user-authored paths.
        // Each bool records whether a sibling follows at that tree level.
        while let Some((row, branches)) = pending.pop() {
            self.rows.push(row);
            self.prefixes.push(tree_prefix(&branches));
            match row {
                VisibleRow::Host(_) => self.shown_hosts += 1,
                VisibleRow::Group(index) if self.is_expanded(index) => {
                    let group = &self.groups[index];
                    let children: Vec<_> = group
                        .children
                        .iter()
                        .filter(|&&child| self.groups[child].matched > 0)
                        .copied()
                        .map(VisibleRow::Group)
                        .chain(group.hosts.iter().copied().map(VisibleRow::Host))
                        .collect();
                    for (position, child) in children.iter().enumerate().rev() {
                        let mut branches = branches.clone();
                        branches.push(position + 1 < children.len());
                        pending.push((*child, branches));
                    }
                }
                VisibleRow::Group(_) => {}
            }
        }
        self.rows
            .extend(self.ungrouped.iter().copied().map(VisibleRow::Host));
        self.prefixes.resize(self.rows.len(), String::new());
        self.shown_hosts += self.ungrouped.len();
    }

    pub fn prefix(&self, position: usize) -> &str {
        &self.prefixes[position]
    }
}

fn tree_prefix(branches: &[bool]) -> String {
    let mut prefix = String::new();
    for (index, &has_sibling) in branches.iter().enumerate() {
        prefix.push_str(match (index + 1 == branches.len(), has_sibling) {
            (true, true) => "├─ ",
            (true, false) => "└─ ",
            (false, true) => "│  ",
            (false, false) => "   ",
        });
    }
    prefix
}
