use super::{shell::centered, theme, App, DetailState};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    frame.render_widget(details_paragraph(app, " Details "), area);
}

pub fn render_overlay(frame: &mut Frame, app: &App) {
    let area = centered(frame.area(), 78, 18);
    frame.render_widget(Clear, area);
    frame.render_widget(details_paragraph(app, " Details - Esc to close "), area);
}

fn details_paragraph(app: &App, title: &'static str) -> Paragraph<'static> {
    Paragraph::new(lines(app))
        .style(Style::default().bg(theme::BG).fg(theme::TEXT))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme::ACCENT))
                .title(Span::styled(
                    title,
                    Style::default()
                        .fg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                )),
        )
}

fn lines(app: &App) -> Vec<Line<'static>> {
    let Some(host) = app.selected_host() else {
        return vec![Line::from(Span::styled(
            "No host selected.",
            Style::default().fg(theme::MUTED),
        ))];
    };

    let mut lines = vec![field("Alias", host.alias.clone())];
    match app.selected_detail() {
        Some(DetailState::Ready(detail)) => {
            lines.push(field("HostName", detail.hostname.clone()));
            lines.push(field("User", value_or_dash(detail.user.as_deref())));
            lines.push(field("Port", value_or_dash(detail.port.as_deref())));
            lines.push(field(
                "ProxyJump",
                value_or_dash(detail.proxy_jump.as_deref()),
            ));
            lines.push(field(
                "ProxyCommand",
                value_or_dash(detail.proxy_command.as_deref()),
            ));
            if detail.identity_files.is_empty() {
                lines.push(field("IdentityFile", "-"));
            } else {
                for (index, identity) in detail.identity_files.iter().enumerate() {
                    lines.push(field(
                        if index == 0 { "IdentityFile" } else { "" },
                        identity.clone(),
                    ));
                }
            }
            lines.push(Line::from(""));
            lines.push(field("Destination", detail.destination()));
        }
        Some(DetailState::Failed(error)) => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("Details unavailable: {error}"),
                Style::default().fg(theme::BAD),
            )));
        }
        None => lines.push(Line::from(Span::styled(
            "Details not loaded.",
            Style::default().fg(theme::MUTED),
        ))),
    }
    lines.push(Line::from(""));
    lines.push(field(
        "Config",
        format!("{}:{}", host.source.display(), host.line),
    ));
    lines
}

fn value_or_dash(value: Option<&str>) -> String {
    value
        .filter(|value| !value.is_empty())
        .unwrap_or("-")
        .to_string()
}

fn field(label: &'static str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<13}"), Style::default().fg(theme::MUTED)),
        Span::styled(value.into(), Style::default().fg(theme::TEXT)),
    ])
}
