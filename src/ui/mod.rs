mod detail;
mod host_list;
mod list;
mod shell;
mod theme;

use crate::catalog::{Catalog, Host};
use crate::ssh::{ResolvedHost, SshClient};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use list::{HostList, VisibleRow};
use ratatui::widgets::TableState;
use ratatui::DefaultTerminal;
use std::collections::HashMap;

pub enum Outcome {
    Quit,
    Connect(Connection),
}

/// A connection the user asked for: the alias plus the number of `-v` flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    pub alias: String,
    pub verbosity: u8,
}

/// `v` cycles through no flag, `-v`, `-vv` and `-vvv`, OpenSSH's maximum.
const VERBOSITY_LEVELS: u8 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    Quit,
    Connect(String),
    Resolve(String),
    Reload,
}

#[derive(Debug, Clone)]
pub enum DetailState {
    Ready(ResolvedHost),
    Failed(String),
}

pub struct App {
    pub catalog: Catalog,
    pub selected: usize,
    pub filter: String,
    pub filter_focused: bool,
    pub show_help: bool,
    pub show_detail_overlay: bool,
    pub show_warnings: bool,
    pub warning_index: usize,
    pub verbosity: u8,
    pub status: Option<String>,
    details: HashMap<String, DetailState>,
    list: HostList,
    table_state: TableState,
}

impl App {
    fn new(catalog: Catalog) -> Self {
        Self {
            list: HostList::new(&catalog),
            catalog,
            selected: 0,
            filter: String::new(),
            filter_focused: false,
            show_help: false,
            show_detail_overlay: false,
            show_warnings: false,
            warning_index: 0,
            verbosity: 0,
            status: None,
            details: HashMap::new(),
            table_state: TableState::default(),
        }
    }

    fn selected_row(&self) -> Option<VisibleRow> {
        self.list.rows.get(self.selected).copied()
    }

    pub fn selected_host(&self) -> Option<&Host> {
        match self.selected_row()? {
            VisibleRow::Host(index) => self.catalog.hosts().get(index),
            VisibleRow::Group(_) => None,
        }
    }

    fn selected_group(&self) -> Option<&list::Group> {
        match self.selected_row()? {
            VisibleRow::Group(index) => self.list.groups.get(index),
            VisibleRow::Host(_) => None,
        }
    }

    pub fn selected_detail(&self) -> Option<&DetailState> {
        self.selected_host()
            .and_then(|host| self.details.get(&host.alias))
    }

    fn selected_alias(&self) -> Option<String> {
        self.selected_host().map(|host| host.alias.clone())
    }

    /// The `-v` flag the next connection passes to ssh, if any.
    pub fn verbosity_flag(&self) -> Option<String> {
        (self.verbosity > 0).then(|| format!("-{}", "v".repeat(usize::from(self.verbosity))))
    }

    fn move_selection(&mut self, delta: isize) -> Option<Action> {
        let length = self.list.rows.len();
        if length == 0 {
            self.selected = 0;
            return None;
        }
        self.selected = (self.selected + length).saturating_add_signed(delta) % length;
        self.resolve_action()
    }

    fn move_page(&mut self, delta: isize) -> Option<Action> {
        let length = self.list.rows.len();
        if length == 0 {
            self.selected = 0;
            return None;
        }
        self.selected = if delta < 0 {
            self.selected.saturating_sub(10)
        } else {
            self.selected.saturating_add(10).min(length - 1)
        };
        self.resolve_action()
    }

    fn resolve_action(&self) -> Option<Action> {
        let alias = self.selected_alias()?;
        (!self.details.contains_key(&alias)).then_some(Action::Resolve(alias))
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Some(Action::Quit);
        }
        if self.show_help {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('?')) {
                self.show_help = false;
            }
            return None;
        }
        if self.show_warnings {
            match key.code {
                KeyCode::Esc | KeyCode::Char('w') => self.show_warnings = false,
                KeyCode::Char('j') | KeyCode::Down | KeyCode::Right => {
                    self.warning_index = (self.warning_index + 1)
                        .min(self.catalog.warnings().len().saturating_sub(1));
                }
                KeyCode::Char('k') | KeyCode::Up | KeyCode::Left => {
                    self.warning_index = self.warning_index.saturating_sub(1);
                }
                _ => {}
            }
            return None;
        }
        if self.show_detail_overlay {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('i')) {
                self.show_detail_overlay = false;
            }
            return None;
        }
        if self.filter_focused {
            return self.handle_search_key(key);
        }

        match key.code {
            KeyCode::Char('q') => Some(Action::Quit),
            KeyCode::Char('?') => {
                self.show_help = true;
                None
            }
            KeyCode::Char('i') if self.selected_host().is_some() => {
                self.show_detail_overlay = true;
                None
            }
            KeyCode::Char('w') => {
                self.show_warnings = true;
                self.warning_index = 0;
                None
            }
            KeyCode::Char('/') => {
                self.filter_focused = true;
                None
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::PageDown => self.move_page(1),
            KeyCode::PageUp => self.move_page(-1),
            KeyCode::Home => {
                self.selected = 0;
                self.resolve_action()
            }
            KeyCode::End => {
                self.selected = self.list.rows.len().saturating_sub(1);
                self.resolve_action()
            }
            KeyCode::Char('r') => Some(Action::Reload),
            KeyCode::Char('v') => {
                self.verbosity = (self.verbosity + 1) % VERBOSITY_LEVELS;
                None
            }
            KeyCode::Char(' ') | KeyCode::Enter if self.selected_group().is_some() => {
                if let Some(VisibleRow::Group(index)) = self.selected_row() {
                    self.list.toggle(index);
                }
                None
            }
            KeyCode::Enter => self.selected_alias().map(Action::Connect),
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.refresh_filter();
                self.resolve_action()
            }
            _ => None,
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Esc => {
                self.filter.clear();
                self.filter_focused = false;
            }
            KeyCode::Enter => {
                self.filter_focused = false;
                return self.resolve_action();
            }
            KeyCode::Backspace => {
                self.filter.pop();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.filter.push(character);
            }
            _ => return None,
        }
        self.refresh_filter();
        self.resolve_action()
    }

    fn refresh_filter(&mut self) {
        self.list.filter(&self.catalog, &self.filter);
        self.selected = if self.filter.trim().is_empty() {
            0
        } else {
            self.list
                .rows
                .iter()
                .position(|row| matches!(row, VisibleRow::Host(_)))
                .unwrap_or(0)
        };
        self.table_state = TableState::default();
    }

    fn set_detail(&mut self, alias: String, detail: Result<ResolvedHost>) {
        let state = match detail {
            Ok(detail) => DetailState::Ready(detail),
            Err(error) => DetailState::Failed(format!("{error:#}")),
        };
        self.details.insert(alias, state);
    }

    fn replace_catalog(&mut self, catalog: Catalog) {
        let selected_alias = self.selected_alias().map(|alias| alias.to_lowercase());
        let selected_group = self.selected_group().map(|group| group.name.clone());
        let mut list = HostList::new(&catalog);
        list.restore_expansion(&self.list);
        list.filter(&catalog, &self.filter);
        self.selected = list
            .rows
            .iter()
            .position(|row| match row {
                VisibleRow::Host(index) => selected_alias
                    .as_ref()
                    .is_some_and(|alias| catalog.hosts()[*index].alias.to_lowercase() == *alias),
                VisibleRow::Group(index) => {
                    selected_group.as_ref() == Some(&list.groups[*index].name)
                }
            })
            .unwrap_or(0);
        self.list = list;
        self.catalog = catalog;
        self.table_state = TableState::default();
        self.details.clear();
    }
}

pub fn run(mut terminal: DefaultTerminal, catalog: Catalog, ssh: &SshClient) -> Result<Outcome> {
    let mut app = App::new(catalog);
    resolve_selected(&mut app, ssh);

    loop {
        terminal.draw(|frame| shell::render(frame, &mut app))?;
        let Event::Key(key) = event::read()? else {
            continue;
        };
        let Some(action) = app.handle_key(key) else {
            continue;
        };
        match action {
            Action::Quit => return Ok(Outcome::Quit),
            Action::Connect(alias) => {
                return Ok(Outcome::Connect(Connection {
                    alias,
                    verbosity: app.verbosity,
                }))
            }
            Action::Resolve(alias) => resolve_alias(&mut app, ssh, alias),
            Action::Reload => {
                let path = app.catalog.config_path().to_path_buf();
                match Catalog::load(&path) {
                    Ok(catalog) => {
                        let count = catalog.hosts().len();
                        app.replace_catalog(catalog);
                        app.status = Some(format!("reloaded {count} host(s)"));
                        resolve_selected(&mut app, ssh);
                    }
                    Err(error) => app.status = Some(format!("reload failed: {error}")),
                }
            }
        }
    }
}

fn resolve_selected(app: &mut App, ssh: &SshClient) {
    if let Some(alias) = app.selected_alias() {
        if !app.details.contains_key(&alias) {
            resolve_alias(app, ssh, alias);
        }
    }
}

fn resolve_alias(app: &mut App, ssh: &SshClient, alias: String) {
    let detail = ssh.resolve(&alias);
    app.set_detail(alias, detail);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Host, HostHints};
    use std::path::PathBuf;

    fn host(alias: &str, hostname: &str) -> Host {
        Host {
            alias: alias.into(),
            source: PathBuf::from("/home/tester/.ssh/config"),
            line: 1,
            hints: HostHints {
                hostname: Some(hostname.into()),
                ..HostHints::default()
            },
            metadata: Default::default(),
        }
    }

    fn app() -> App {
        App::new(Catalog::from_hosts_for_test(vec![
            host("production", "api.internal"),
            host("staging", "staging.internal"),
        ]))
    }

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn grouped_app() -> App {
        let mut api = host("api", "api.internal");
        api.metadata.group = Some("Work".into());
        api.metadata.tags = vec!["critical".into()];
        let mut backup = host("backup", "backup.internal");
        backup.metadata.group = Some("Personal".into());
        let mut staging = host("staging", "staging.internal");
        staging.metadata.group = Some("Work".into());
        App::new(Catalog::from_hosts_for_test(vec![
            host("standalone", "localhost"),
            api,
            backup,
            staging,
        ]))
    }

    #[test]
    fn groups_start_collapsed_in_declaration_order_above_ungrouped_hosts() {
        let mut app = grouped_app();
        assert_eq!(
            app.list.rows,
            [
                VisibleRow::Group(0),
                VisibleRow::Group(1),
                VisibleRow::Host(0)
            ]
        );
        assert_eq!(app.selected_group().unwrap().name, "Work");
        assert!(app.selected_host().is_none());
        assert!(app.resolve_action().is_none());
        app.handle_key(press(KeyCode::Char('i')));
        assert!(!app.show_detail_overlay);

        assert_eq!(app.handle_key(press(KeyCode::Char(' '))), None);
        assert_eq!(
            app.list.rows,
            [
                VisibleRow::Group(0),
                VisibleRow::Host(1),
                VisibleRow::Host(3),
                VisibleRow::Group(1),
                VisibleRow::Host(0)
            ]
        );
        assert_eq!(app.handle_key(press(KeyCode::Enter)), None);
        assert_eq!(app.list.rows.len(), 3);
        app.handle_key(press(KeyCode::Enter));
        assert_eq!(
            app.handle_key(press(KeyCode::Down)),
            Some(Action::Resolve("api".into()))
        );
        assert_eq!(app.handle_key(press(KeyCode::Char(' '))), None);
        assert_eq!(
            app.handle_key(press(KeyCode::Enter)),
            Some(Action::Connect("api".into()))
        );
    }

    #[test]
    fn navigation_skips_collapsed_children_and_wraps_without_resolving_groups() {
        let mut app = grouped_app();
        assert_eq!(app.handle_key(press(KeyCode::Down)), None);
        assert_eq!(app.selected_group().unwrap().name, "Personal");
        assert_eq!(
            app.handle_key(press(KeyCode::Down)),
            Some(Action::Resolve("standalone".into()))
        );
        assert_eq!(app.handle_key(press(KeyCode::Down)), None);
        assert_eq!(app.selected_group().unwrap().name, "Work");
        assert_eq!(
            app.handle_key(press(KeyCode::End)),
            Some(Action::Resolve("standalone".into()))
        );
        assert_eq!(app.handle_key(press(KeyCode::Home)), None);
    }

    #[test]
    fn search_reveals_matches_and_restores_expansion_even_after_toggling_search_groups() {
        let mut app = grouped_app();
        app.handle_key(press(KeyCode::Char(' '))); // Work stays open after searching.
        app.handle_key(press(KeyCode::Char('/')));
        for character in "backup".chars() {
            app.handle_key(press(KeyCode::Char(character)));
        }
        assert_eq!(app.selected_host().unwrap().alias, "backup");
        assert_eq!(app.list.rows, [VisibleRow::Group(1), VisibleRow::Host(2)]);
        app.handle_key(press(KeyCode::Enter));
        app.handle_key(press(KeyCode::Home));
        app.handle_key(press(KeyCode::Char(' ')));
        assert_eq!(app.list.rows, [VisibleRow::Group(1)]);
        app.handle_key(press(KeyCode::Esc));
        assert!(app.list.is_expanded(0));
        assert!(!app.list.is_expanded(1));
        assert_eq!(app.list.shown_hosts, 3);
    }

    #[test]
    fn search_matches_group_names_and_tags_and_space_remains_search_text() {
        let mut app = grouped_app();
        app.filter = "critical".into();
        app.refresh_filter();
        assert_eq!(app.selected_host().unwrap().alias, "api");
        assert_eq!(app.list.shown_hosts, 1);
        app.filter = "Work".into();
        app.refresh_filter();
        assert_eq!(app.list.shown_hosts, 2);
        app.filter_focused = true;
        app.handle_key(press(KeyCode::Char(' ')));
        assert_eq!(app.filter, "Work ");
        assert!(app.list.is_expanded(0));
        app.filter = "does-not-exist".into();
        app.refresh_filter();
        assert!(app.list.rows.is_empty());
        assert_eq!(app.handle_key(press(KeyCode::Enter)), None);
        assert_eq!(app.handle_key(press(KeyCode::Enter)), None);
    }

    #[test]
    fn reload_preserves_known_groups_and_selection_and_removes_stale_rows() {
        let mut app = grouped_app();
        app.handle_key(press(KeyCode::Char(' ')));
        app.handle_key(press(KeyCode::Down));
        app.details
            .insert("api".into(), DetailState::Failed("old".into()));
        let mut hosts = app.catalog.hosts().to_vec();
        hosts.swap(1, 2); // Group indices can change after reload.
        app.replace_catalog(Catalog::from_hosts_for_test(hosts));
        assert_eq!(app.selected_host().unwrap().alias, "api");
        assert!(app.selected_detail().is_none());
        assert!(!app.list.is_expanded(0));
        assert!(app.list.is_expanded(1));

        app.replace_catalog(Catalog::from_hosts_for_test(vec![]));
        assert!(app.selected_host().is_none());
        assert_eq!(app.handle_key(press(KeyCode::Down)), None);
        assert_eq!(app.handle_key(press(KeyCode::Enter)), None);
    }

    #[test]
    fn warning_overlay_consumes_connection_keys_and_closes_with_escape() {
        let mut app = app();
        app.handle_key(press(KeyCode::Char('w')));
        assert!(app.show_warnings);
        assert_eq!(app.handle_key(press(KeyCode::Enter)), None);
        app.handle_key(press(KeyCode::Down));
        assert_eq!(app.warning_index, 0);
        app.handle_key(press(KeyCode::Esc));
        assert!(!app.show_warnings);
        assert_eq!(
            app.handle_key(press(KeyCode::Enter)),
            Some(Action::Connect("production".into()))
        );
    }

    #[test]
    fn reload_uses_the_catalogs_case_insensitive_alias_identity() {
        let mut app = App::new(Catalog::from_hosts_for_test(vec![
            host("before", "localhost"),
            host("Ápi", "api.internal"),
        ]));
        app.selected = 1;
        app.replace_catalog(Catalog::from_hosts_for_test(vec![
            host("before", "localhost"),
            host("áPI", "api.internal"),
        ]));
        assert_eq!(app.selected_host().unwrap().alias, "áPI");
    }

    #[test]
    fn enter_connects_to_the_selected_alias() {
        let mut app = app();
        assert_eq!(
            app.handle_key(press(KeyCode::Enter)),
            Some(Action::Connect("production".into()))
        );
    }

    #[test]
    fn navigation_wraps_and_requests_uncached_details() {
        let mut app = app();
        assert_eq!(
            app.handle_key(press(KeyCode::Char('k'))),
            Some(Action::Resolve("staging".into()))
        );
        assert_eq!(app.selected, 1);
        assert_eq!(
            app.handle_key(press(KeyCode::Char('j'))),
            Some(Action::Resolve("production".into()))
        );
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn search_filters_by_hostname_and_escape_clears_it() {
        let mut app = app();
        app.handle_key(press(KeyCode::Char('/')));
        for character in "stg".chars() {
            app.handle_key(press(KeyCode::Char(character)));
        }
        assert!(app.filter_focused);
        assert_eq!(app.selected_host().unwrap().alias, "staging");

        app.handle_key(press(KeyCode::Esc));
        assert!(!app.filter_focused);
        assert!(app.filter.is_empty());
        assert_eq!(app.selected_host().unwrap().alias, "production");
    }

    #[test]
    fn v_cycles_ssh_verbosity_only_outside_search() {
        let mut app = app();
        assert_eq!(app.verbosity_flag(), None);
        for expected in ["-v", "-vv", "-vvv"] {
            assert_eq!(app.handle_key(press(KeyCode::Char('v'))), None);
            assert_eq!(app.verbosity_flag().as_deref(), Some(expected));
        }
        app.handle_key(press(KeyCode::Char('v')));
        assert_eq!(app.verbosity, 0);

        app.handle_key(press(KeyCode::Char('/')));
        app.handle_key(press(KeyCode::Char('v')));
        assert_eq!(app.verbosity, 0);
        assert_eq!(app.filter, "v");
    }

    #[test]
    fn detail_overlay_consumes_enter_instead_of_connecting() {
        let mut app = app();
        app.handle_key(press(KeyCode::Char('i')));
        assert!(app.show_detail_overlay);
        assert_eq!(app.handle_key(press(KeyCode::Enter)), None);
        app.handle_key(press(KeyCode::Esc));
        assert!(!app.show_detail_overlay);
    }
}
