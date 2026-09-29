use super::{detail, host_list, theme, App};
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

const DETAIL_BREAKPOINT: u16 = 100;
const VERSION_LABEL: &str = concat!("v", env!("CARGO_PKG_VERSION"));

pub fn render(frame: &mut Frame, app: &mut App) {
    frame.render_widget(
        Block::default().style(Style::default().bg(theme::BG).fg(theme::TEXT)),
        frame.area(),
    );
    let [header, body, status, hints] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    render_header(frame, header);
    if body.width >= DETAIL_BREAKPOINT {
        let [list, details] =
            Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)])
                .areas(body);
        host_list::render(frame, app, list);
        detail::render(frame, app, details);
    } else {
        host_list::render(frame, app, body);
    }
    render_status(frame, app, status);
    render_hints(frame, app, hints);

    if app.show_detail_overlay {
        detail::render_overlay(frame, app);
    }
    if app.show_help {
        render_help(frame);
    }
    if app.show_warnings {
        render_warnings(frame, app);
    }
}

fn render_header(frame: &mut Frame, area: Rect) {
    // Reserve the version before optional branding. The embedded package
    // version agrees with --version and identifies the running binary even
    // when the checkout or installed executable has since been replaced.
    let version_width = u16::try_from(VERSION_LABEL.len())
        .unwrap_or(u16::MAX)
        .min(area.width);
    let [identity, version] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(version_width)]).areas(area);
    let brand = " porteiro ";
    let description = " OpenSSH config hosts";
    let mut spans = Vec::with_capacity(2);
    if usize::from(identity.width) >= brand.len() {
        spans.push(Span::styled(
            brand,
            Style::default().add_modifier(Modifier::BOLD),
        ));
    }
    if usize::from(identity.width) >= brand.len() + description.len() {
        spans.push(Span::raw(description));
    }
    let style = Style::default().fg(theme::HEADER_FG).bg(theme::HEADER_BG);
    frame.render_widget(Paragraph::new(Line::from(spans)).style(style), identity);
    frame.render_widget(
        Paragraph::new(VERSION_LABEL).style(style.add_modifier(Modifier::BOLD)),
        version,
    );
}

fn render_status(frame: &mut Frame, app: &App, area: Rect) {
    let warning = match app.catalog.warnings().len() {
        0 => String::new(),
        count => format!(" | {count} warning(s) [w]"),
    };
    let context = if app.filter_focused {
        format!("search: {}_", app.filter)
    } else if !app.filter.is_empty() {
        format!("search: {}", app.filter)
    } else if let Some(status) = &app.status {
        status.clone()
    } else {
        app.catalog.config_path().display().to_string()
    };
    let verbose = app
        .verbosity_flag()
        .map(|flag| format!(" | ssh {flag}"))
        .unwrap_or_default();
    let text = format!(
        " {} host(s){warning}{verbose} | {context}",
        app.catalog.hosts().len()
    );
    frame.render_widget(
        Paragraph::new(text).style(Style::default().bg(theme::BG).fg(theme::MUTED)),
        area,
    );
}

fn render_hints(frame: &mut Frame, app: &App, area: Rect) {
    let line = if app.filter_focused {
        Line::from(" type to search   Enter accept   Esc clear")
    } else {
        let mut spans = vec![Span::raw(if app.selected_group().is_some() {
            " j/k move  Space/Enter expand/collapse  / search"
        } else {
            " j/k move  / search  Enter connect"
        })];
        if app.selected_host().is_some() {
            if let Some(flag) = app.verbosity_flag() {
                spans.push(Span::styled(
                    format!(" {flag}"),
                    Style::default()
                        .fg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ));
            }
        }
        spans.push(Span::raw(
            "  i details  r reload  v verbose  w warnings  ? help  q quit",
        ));
        Line::from(spans)
    };
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(theme::BG).fg(theme::MUTED)),
        area,
    );
}

fn render_help(frame: &mut Frame) {
    let area = centered(frame.area(), 62, 18);
    frame.render_widget(Clear, area);
    let help = Paragraph::new(vec![
        Line::from("j / k, arrows    move selection"),
        Line::from("PgUp / PgDn      move by page"),
        Line::from("Home / End       first / last row"),
        Line::from("/                focus search"),
        Line::from("Enter            accept search, toggle group or connect"),
        Line::from("Space            expand / collapse selected group"),
        Line::from("Esc              clear search or close overlay"),
        Line::from("i                technical details"),
        Line::from("r                reload SSH config"),
        Line::from("w                view config warnings"),
        Line::from("v                cycle ssh verbosity: -v, -vv, -vvv"),
        Line::from("?                toggle help"),
        Line::from("q / Ctrl-C       quit"),
    ])
    .style(Style::default().bg(theme::BG).fg(theme::TEXT))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme::IDENT))
            .title(Span::styled(
                " Help ",
                Style::default()
                    .fg(theme::IDENT)
                    .add_modifier(Modifier::BOLD),
            )),
    );
    frame.render_widget(help, area);
}

fn render_warnings(frame: &mut Frame, app: &App) {
    let area = centered(frame.area(), 90, 18);
    frame.render_widget(Clear, area);
    let warnings = app.catalog.warnings();
    let index = app.warning_index.min(warnings.len().saturating_sub(1));
    let title = if warnings.is_empty() {
        " Warnings - Esc to close ".to_string()
    } else {
        format!(
            " Warning {} / {} - j next / k previous - Esc close ",
            index + 1,
            warnings.len()
        )
    };
    let message = warnings
        .get(index)
        .map(String::as_str)
        .unwrap_or("No configuration warnings.");
    frame.render_widget(
        Paragraph::new(message)
            .wrap(Wrap { trim: false })
            .style(Style::default().bg(theme::BG).fg(theme::TEXT))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(theme::ACCENT))
                    .title(title),
            ),
        area,
    );
}

pub(crate) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2)).max(1);
    let height = height.min(area.height.saturating_sub(2)).max(1);
    let [horizontal] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    let [rect] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(horizontal);
    rect
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Catalog, Host, HostHints};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    fn app() -> App {
        let host = |alias: &str, hostname: &str| Host {
            alias: alias.into(),
            source: PathBuf::from("/home/tester/.ssh/config"),
            line: 1,
            hints: HostHints {
                hostname: Some(hostname.into()),
                ..HostHints::default()
            },
            metadata: Default::default(),
        };
        App::new(Catalog::from_hosts_for_test(vec![
            host("production", "api.internal"),
            host("staging", "staging.internal"),
        ]))
    }

    fn rendered(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .chunks(usize::from(width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>() + "\n")
            .collect()
    }

    #[test]
    fn wide_terminals_show_columns_details_pane_and_hints() {
        let screen = rendered(&mut app(), 120, 24);
        assert!(screen.contains("porteiro"));
        assert!(screen.contains("Hosts - 2 shown / 2 total"));
        assert!(screen.contains("HostName") && screen.contains("api.internal"));
        assert!(screen.contains("staging.internal"));
        assert!(screen.contains("Details"));
        assert!(screen.contains("2 host(s) | /home/tester/.ssh/config"));
        assert!(screen.contains("j/k move"));
    }

    #[test]
    fn port_column_shows_default_explicit_and_unknown_values() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config");
        std::fs::write(
            &config,
            r#"
Host plain-port explicit-port unknown-port
Host explicit-port
  Port 2222
Match originalhost unknown-port exec "exit 0"
  Port 2200
"#,
        )
        .unwrap();
        let mut app = App::new(Catalog::load(&config).unwrap());
        let screen = rendered(&mut app, 180, 24);
        let header = line_containing(&screen, "HostName");
        let port_column = header[..header.find("Port").unwrap()].chars().count();

        for (alias, expected) in [
            ("plain-port", "22"),
            ("explicit-port", "2222"),
            ("unknown-port", "-"),
        ] {
            let row = screen
                .lines()
                .find(|row| {
                    row.split_once("││")
                        .is_some_and(|(list, _)| list.contains(alias))
                })
                .unwrap();
            let port: String = row.chars().skip(port_column).take(6).collect();
            assert_eq!(port.trim(), expected, "{row}");
        }
    }

    #[test]
    fn narrow_terminals_drop_optional_columns_and_the_details_pane() {
        let screen = rendered(&mut app(), 40, 12);
        assert!(screen.contains("production") && screen.contains("staging"));
        assert!(!screen.contains("HostName"));
        assert!(!screen.contains("Details"));
    }

    #[test]
    fn header_keeps_the_running_version_visible_when_space_is_tight() {
        for width in [120, 40, 18, 12, VERSION_LABEL.len() as u16] {
            let screen = rendered(&mut app(), width, 12);
            let header = screen.lines().next().unwrap();
            assert!(
                header.ends_with(VERSION_LABEL),
                "{width} columns: {header:?}"
            );
            if width < 40 {
                assert!(!header.contains("OpenSSH config hosts"));
            }
        }
    }

    #[test]
    fn overlays_leave_the_version_visible() {
        let mut app = app();
        app.show_help = true;
        assert!(rendered(&mut app, 120, 24)
            .lines()
            .next()
            .unwrap()
            .ends_with(VERSION_LABEL));
        app.show_help = false;
        app.show_detail_overlay = true;
        assert!(rendered(&mut app, 120, 24)
            .lines()
            .next()
            .unwrap()
            .ends_with(VERSION_LABEL));
        app.show_detail_overlay = false;
        app.show_warnings = true;
        assert!(rendered(&mut app, 120, 24)
            .lines()
            .next()
            .unwrap()
            .ends_with(VERSION_LABEL));
    }

    #[test]
    fn overlays_render_on_top_of_the_list() {
        let mut app = app();
        app.show_detail_overlay = true;
        assert!(rendered(&mut app, 120, 24).contains("Details - Esc to close"));

        app.show_detail_overlay = false;
        app.show_help = true;
        let screen = rendered(&mut app, 120, 24);
        assert!(screen.contains(" Help ") && screen.contains("toggle help"));
    }

    fn line_containing(screen: &str, needle: &str) -> String {
        screen
            .lines()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("no line contains {needle:?}:\n{screen}"))
            .to_string()
    }

    #[test]
    fn only_the_selected_row_carries_the_marker() {
        let mut app = app();
        let screen = rendered(&mut app, 120, 24);
        assert!(line_containing(&screen, "api.internal").starts_with("│›"));
        assert!(line_containing(&screen, "staging.internal").starts_with("│ "));

        app.selected = 1;
        let screen = rendered(&mut app, 120, 24);
        assert!(line_containing(&screen, "api.internal").starts_with("│ "));
        assert!(line_containing(&screen, "staging.internal").starts_with("│›"));
    }

    #[test]
    fn verbosity_badges_the_selected_row_and_the_enter_hint() {
        let mut app = app();
        let screen = rendered(&mut app, 120, 24);
        assert!(!screen.contains("| ssh -v") && !screen.contains("connect -v"));

        app.verbosity = 2;
        let screen = rendered(&mut app, 120, 24);
        let selected_row = line_containing(&screen, "api.internal");
        assert!(selected_row.contains("production   -vv "), "{selected_row}");
        assert!(!line_containing(&screen, "staging.internal").contains("-vv"));
        assert!(screen.contains("Enter connect -vv  i details"));
        assert!(screen.contains("2 host(s) | ssh -vv |"));

        app.show_help = true;
        assert!(rendered(&mut app, 120, 24).contains("cycle ssh verbosity"));
    }

    #[test]
    fn search_and_empty_results_are_explained_in_place() {
        let mut app = app();
        app.filter_focused = true;
        app.filter = "nothing-matches".into();
        app.refresh_filter();
        let screen = rendered(&mut app, 120, 24);
        assert!(screen.contains("search: nothing-matches_"));
        assert!(screen.contains("Nothing matches the current search"));
        assert!(screen.contains("type to search"));
    }

    #[test]
    fn grouped_rows_render_counts_badges_colors_and_group_specific_hints() {
        let mut app = app();
        let mut hosts = app.catalog.hosts().to_vec();
        hosts[0].metadata = crate::catalog::HostMetadata {
            group: vec!["Work".into()],
            tags: vec!["prod".into(), "api".into()],
            color: Some(crate::catalog::AccentColor::Red),
            hidden: false,
        };
        app.replace_catalog(Catalog::from_hosts_for_test(hosts));
        app.selected = 0;
        app.verbosity = 2;
        let screen = rendered(&mut app, 96, 20);
        assert!(screen.contains("▸ Work (1)"));
        assert!(!screen.contains("production"));
        assert!(screen.contains("staging"));
        assert!(screen.contains("Space/Enter expand/collapse"));
        assert!(!line_containing(&screen, "Work (1)").contains("-vv"));

        app.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(' '),
            crossterm::event::KeyModifiers::NONE,
        ));
        app.selected = 1;
        let screen = rendered(&mut app, 96, 20);
        assert!(screen.contains("▾ Work (1)"));
        let row = line_containing(&screen, "production");
        assert!(row.contains(" prod ") && row.contains(" api "), "{row}");
        assert!(row.contains("-vv"));

        let mut terminal = Terminal::new(TestBackend::new(96, 20)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let cells = &terminal.backend().buffer().content;
        assert!(cells
            .iter()
            .any(|cell| cell.symbol() == "p" && cell.fg == ratatui::style::Color::LightRed));
        assert!(cells
            .iter()
            .any(|cell| cell.symbol() == "▾" && cell.fg == ratatui::style::Color::LightRed));
    }

    #[test]
    fn narrow_grouped_rows_preserve_aliases_and_details_retain_all_tags() {
        let mut app = app();
        let mut hosts = app.catalog.hosts().to_vec();
        hosts[0].metadata.group = vec!["Equipe 東京".into()];
        hosts[0].metadata.tags = vec!["produção".into(), "database".into()];
        app.replace_catalog(Catalog::from_hosts_for_test(hosts));
        app.filter = "production".into();
        app.refresh_filter();
        let screen = rendered(&mut app, 24, 12);
        assert!(screen.contains("production"));
        assert!(!screen.contains("produção"));
        assert!(screen.contains("+2"));
        app.show_detail_overlay = true;
        let screen = rendered(&mut app, 100, 24);
        assert!(screen.contains("Tags") && screen.contains("produção, database"));
        for (width, height) in [(1, 1), (4, 3), (12, 5)] {
            rendered(&mut app, width, height);
        }
    }

    #[test]
    fn tag_badges_keep_their_colors_and_padding_on_selected_and_colored_rows() {
        let mut app = app();
        let mut hosts = app.catalog.hosts().to_vec();
        hosts[0].metadata.color = Some(crate::catalog::AccentColor::Red);
        hosts[1].metadata.color = Some(crate::catalog::AccentColor::Blue);
        for host in &mut hosts {
            host.metadata.tags = vec!["prod".into(), "api".into()];
        }
        app.replace_catalog(Catalog::from_hosts_for_test(hosts));
        for selected in [0, 1] {
            app.selected = selected;
            let mut terminal = Terminal::new(TestBackend::new(96, 20)).unwrap();
            terminal.draw(|frame| render(frame, &mut app)).unwrap();
            let rows: Vec<_> = terminal
                .backend()
                .buffer()
                .content
                .chunks(96)
                .filter(|row| row.iter().any(|cell| cell.bg == theme::TAG_BG))
                .collect();
            assert_eq!(rows.len(), 2);
            for row in rows {
                let badge_cells: Vec<_> =
                    row.iter().filter(|cell| cell.bg == theme::TAG_BG).collect();
                assert_eq!(
                    badge_cells
                        .iter()
                        .map(|cell| cell.symbol())
                        .collect::<String>(),
                    " prod  api "
                );
                assert!(badge_cells.iter().all(
                    |cell| cell.fg == theme::TAG_FG && !cell.modifier.contains(Modifier::BOLD)
                ));
                let first = row
                    .iter()
                    .position(|cell| cell.bg == theme::TAG_BG)
                    .unwrap();
                assert_ne!(row[first - 1].bg, theme::TAG_BG);
                assert_ne!(row[first + 6].bg, theme::TAG_BG); // Gap between badges.
            }
        }
    }

    #[test]
    fn nested_tree_renders_connectors_descendant_counts_and_omits_hidden_groups() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = temp.path().join("config");
        std::fs::write(&config, "Host api-one api-two\n# porteiro: group_path=Production/APIs\nHost database\n# porteiro: group_path=Production/Data\nHost bastion\n# porteiro: group=Production\nHost other\n# porteiro: group_path=Staging/APIs\nHost secret\n# porteiro: group_path=Hidden/Private hidden=true\n").unwrap();
        let mut app = App::new(Catalog::load(&config).unwrap());
        let screen = rendered(&mut app, 100, 24);
        assert!(screen.contains("▸ Production (4)"));
        assert!(screen.contains("▸ Staging (1)"));
        assert!(!screen.contains("Hidden") && !screen.contains("secret"));

        app.filter = "Production".into();
        app.refresh_filter();
        let screen = rendered(&mut app, 100, 24);
        for branch in [
            "├─ ▾ APIs (2)",
            "│  ├─ api-one",
            "│  └─ api-two",
            "├─ ▾ Data (1)",
            "│  └─ database",
            "└─ bastion",
        ] {
            assert!(screen.contains(branch), "missing {branch:?}:\n{screen}");
        }
        assert!(!screen.contains("Staging"));

        app.filter = "api-one".into();
        app.refresh_filter();
        let screen = rendered(&mut app, 100, 24);
        assert!(screen.contains("▾ Production (1/4)"));
        assert!(screen.contains("└─ ▾ APIs (1/2)"));
        assert!(screen.contains("   └─ api-one"));
        assert!(!screen.contains("│  └─ api-one"));
        app.show_detail_overlay = true;
        assert!(rendered(&mut app, 100, 24).contains("Production / APIs"));
    }

    #[test]
    fn an_all_hidden_catalog_explains_the_empty_list_and_cannot_connect() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = temp.path().join("config");
        std::fs::write(
            &config,
            "Host secret\n# porteiro: hidden=true group_path=Hidden/Private\n",
        )
        .unwrap();
        let mut app = App::new(Catalog::load(&config).unwrap());
        let screen = rendered(&mut app, 100, 24);
        assert!(screen.contains("No visible Host aliases"));
        assert!(screen.contains("0 shown / 0 total"));
        assert_eq!(
            app.handle_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE
            )),
            None
        );
    }

    #[test]
    fn deep_group_paths_keep_the_selected_alias_readable_in_a_narrow_terminal() {
        let mut app = app();
        let mut hosts = app.catalog.hosts().to_vec();
        hosts[0].metadata.group = (0..64).map(|index| format!("東京 {index}")).collect();
        app.replace_catalog(Catalog::from_hosts_for_test(hosts));
        app.filter = "production".into();
        app.refresh_filter();
        app.verbosity = 3;
        let screen = rendered(&mut app, 36, 12);
        let row = line_containing(&screen, "production");
        assert!(row.contains("-vvv"));
        assert!(row.contains('…'));
        for (width, height) in [(1, 1), (4, 3), (12, 5)] {
            rendered(&mut app, width, height);
        }
    }

    #[test]
    fn viewport_is_preserved_while_moving_within_the_visible_page() {
        let mut app = app();
        let template = app.catalog.hosts()[0].clone();
        let hosts = (0..40)
            .map(|index| {
                let mut host = template.clone();
                host.alias = format!("host-{index:02}");
                host
            })
            .collect();
        app.replace_catalog(Catalog::from_hosts_for_test(hosts));
        app.selected = 30;
        rendered(&mut app, 60, 12);
        let offset = app.table_state.offset();
        assert!(offset > 0);
        app.selected = 29;
        rendered(&mut app, 60, 12);
        assert_eq!(app.table_state.offset(), offset);
    }

    #[test]
    fn warnings_show_source_and_can_be_paged_without_connecting() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = temp.path().join("config");
        std::fs::write(
            &config,
            "Host api\n# porteiro: color=invalid\n# porteiro: unknown=value\n",
        )
        .unwrap();
        let mut app = App::new(Catalog::load(&config).unwrap());
        let key =
            |code| crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE);
        app.handle_key(key(crossterm::event::KeyCode::Char('w')));
        let screen = rendered(&mut app, 100, 24);
        assert!(screen.contains("Warning 1 / 2"));
        assert!(screen.contains("config:2:"));
        assert!(screen.contains("unknown color"));
        app.handle_key(key(crossterm::event::KeyCode::Down));
        let screen = rendered(&mut app, 100, 24);
        assert!(screen.contains("Warning 2 / 2"));
        assert!(screen.contains("config:3:"));
        assert!(screen.contains("unknown metadata key"));
    }

    #[test]
    fn centered_overlay_is_clamped_to_small_terminals() {
        let area = Rect::new(0, 0, 20, 8);
        let overlay = centered(area, 80, 30);
        assert!(overlay.width <= 18);
        assert!(overlay.height <= 6);
        assert!(overlay.x < area.width);
        assert!(overlay.y < area.height);
    }
}
