use super::{detail, host_list, theme, App};
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

const DETAIL_BREAKPOINT: u16 = 100;

pub fn render(frame: &mut Frame, app: &App) {
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
}

fn render_header(frame: &mut Frame, area: Rect) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " porteiro ",
                Style::default()
                    .fg(theme::HEADER_FG)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " OpenSSH config hosts",
                Style::default().fg(theme::HEADER_FG),
            ),
        ]))
        .style(Style::default().bg(theme::HEADER_BG)),
        area,
    );
}

fn render_status(frame: &mut Frame, app: &App, area: Rect) {
    let warning = match app.catalog.warnings().len() {
        0 => String::new(),
        count => format!(" | {count} warning(s)"),
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
    let text = format!(
        " {} host(s){warning} | {context}",
        app.catalog.hosts().len()
    );
    frame.render_widget(
        Paragraph::new(text).style(Style::default().bg(theme::BG).fg(theme::MUTED)),
        area,
    );
}

fn render_hints(frame: &mut Frame, app: &App, area: Rect) {
    let text = if app.filter_focused {
        " type to search   Enter accept   Esc clear"
    } else {
        " j/k move   / search   Enter connect   i details   r reload   ? help   q quit"
    };
    frame.render_widget(
        Paragraph::new(text).style(Style::default().bg(theme::BG).fg(theme::MUTED)),
        area,
    );
}

fn render_help(frame: &mut Frame) {
    let area = centered(frame.area(), 60, 17);
    frame.render_widget(Clear, area);
    let help = Paragraph::new(vec![
        Line::from("j / k, arrows    move selection"),
        Line::from("PgUp / PgDn      move by page"),
        Line::from("Home / End       first / last host"),
        Line::from("/                focus search"),
        Line::from("Enter            accept search or connect"),
        Line::from("Esc              clear search or close overlay"),
        Line::from("i                technical details"),
        Line::from("r                reload SSH config"),
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
        };
        App::new(Catalog::from_hosts_for_test(vec![
            host("production", "api.internal"),
            host("staging", "staging.internal"),
        ]))
    }

    fn rendered(app: &App, width: u16, height: u16) -> String {
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
        let screen = rendered(&app(), 120, 24);
        assert!(screen.contains("porteiro"));
        assert!(screen.contains("Hosts - 2 shown / 2 total"));
        assert!(screen.contains("HostName") && screen.contains("api.internal"));
        assert!(screen.contains("staging.internal"));
        assert!(screen.contains("Details"));
        assert!(screen.contains("2 host(s) | /home/tester/.ssh/config"));
        assert!(screen.contains("j/k move"));
    }

    #[test]
    fn narrow_terminals_drop_optional_columns_and_the_details_pane() {
        let screen = rendered(&app(), 40, 12);
        assert!(screen.contains("production") && screen.contains("staging"));
        assert!(!screen.contains("HostName"));
        assert!(!screen.contains("Details"));
    }

    #[test]
    fn overlays_render_on_top_of_the_list() {
        let mut app = app();
        app.show_detail_overlay = true;
        assert!(rendered(&app, 120, 24).contains("Details - Esc to close"));

        app.show_detail_overlay = false;
        app.show_help = true;
        let screen = rendered(&app, 120, 24);
        assert!(screen.contains(" Help ") && screen.contains("toggle help"));
    }

    #[test]
    fn search_and_empty_results_are_explained_in_place() {
        let mut app = app();
        app.filter_focused = true;
        app.filter = "nothing-matches".into();
        let screen = rendered(&app, 120, 24);
        assert!(screen.contains("search: nothing-matches_"));
        assert!(screen.contains("Nothing matches the current search"));
        assert!(screen.contains("type to search"));
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
