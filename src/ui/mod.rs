mod detail;
mod host_list;
mod shell;
mod theme;

use crate::catalog::{Catalog, Host};
use crate::ssh::{ResolvedHost, SshClient};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
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
    pub verbosity: u8,
    pub status: Option<String>,
    details: HashMap<String, DetailState>,
}

impl App {
    fn new(catalog: Catalog) -> Self {
        Self {
            catalog,
            selected: 0,
            filter: String::new(),
            filter_focused: false,
            show_help: false,
            show_detail_overlay: false,
            verbosity: 0,
            status: None,
            details: HashMap::new(),
        }
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        self.catalog.filtered_indices(&self.filter)
    }

    pub fn selected_host(&self) -> Option<&Host> {
        let visible = self.visible_indices();
        let selected = self.selected.min(visible.len().saturating_sub(1));
        visible
            .get(selected)
            .and_then(|index| self.catalog.hosts().get(*index))
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
        let length = self.visible_indices().len();
        if length == 0 {
            self.selected = 0;
            return None;
        }
        self.selected = (self.selected + length).saturating_add_signed(delta) % length;
        self.resolve_action()
    }

    fn move_page(&mut self, delta: isize) -> Option<Action> {
        let length = self.visible_indices().len();
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
                self.selected = self.visible_indices().len().saturating_sub(1);
                self.resolve_action()
            }
            KeyCode::Char('r') => Some(Action::Reload),
            KeyCode::Char('v') => {
                self.verbosity = (self.verbosity + 1) % VERBOSITY_LEVELS;
                None
            }
            KeyCode::Enter => self.selected_alias().map(Action::Connect),
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.selected = 0;
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
                self.selected = 0;
            }
            KeyCode::Enter => self.filter_focused = false,
            KeyCode::Backspace => {
                self.filter.pop();
                self.selected = 0;
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.filter.push(character);
                self.selected = 0;
            }
            _ => return None,
        }
        self.resolve_action()
    }

    fn set_detail(&mut self, alias: String, detail: Result<ResolvedHost>) {
        let state = match detail {
            Ok(detail) => DetailState::Ready(detail),
            Err(error) => DetailState::Failed(format!("{error:#}")),
        };
        self.details.insert(alias, state);
    }

    fn replace_catalog(&mut self, catalog: Catalog) {
        self.catalog = catalog;
        self.selected = 0;
        self.details.clear();
    }
}

pub fn run(mut terminal: DefaultTerminal, catalog: Catalog, ssh: &SshClient) -> Result<Outcome> {
    let mut app = App::new(catalog);
    resolve_selected(&mut app, ssh);

    loop {
        terminal.draw(|frame| shell::render(frame, &app))?;
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
