use super::{theme, App};
use crate::catalog::Host;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Marker,
    Alias,
    Hostname,
    User,
    Port,
    Source,
}

impl Column {
    const DISPLAY_ORDER: [Self; 6] = [
        Self::Marker,
        Self::Alias,
        Self::Hostname,
        Self::User,
        Self::Port,
        Self::Source,
    ];
    const OPTIONAL_BY_VALUE: [Self; 4] = [Self::Hostname, Self::User, Self::Port, Self::Source];

    fn header(self) -> &'static str {
        match self {
            Self::Marker => "",
            Self::Alias => "Alias",
            Self::Hostname => "HostName",
            Self::User => "User",
            Self::Port => "Port",
            Self::Source => "Config",
        }
    }

    fn width(self) -> u16 {
        match self {
            Self::Marker => 1,
            Self::Alias => 20,
            Self::Hostname => 28,
            Self::User => 14,
            Self::Port => 6,
            Self::Source => 22,
        }
    }

    fn constraint(self) -> Constraint {
        match self {
            Self::Alias => Constraint::Min(self.width()),
            other => Constraint::Length(other.width()),
        }
    }
}

fn visible_columns(area_width: u16) -> Vec<Column> {
    let mut kept = vec![Column::Marker, Column::Alias];
    let cost = |columns: &[Column]| -> u16 {
        columns.iter().map(|column| column.width()).sum::<u16>()
            + columns.len().saturating_sub(1) as u16
            + 2
    };
    for candidate in Column::OPTIONAL_BY_VALUE {
        let mut trial = kept.clone();
        trial.push(candidate);
        if cost(&trial) <= area_width {
            kept = trial;
        }
    }
    Column::DISPLAY_ORDER
        .into_iter()
        .filter(|column| kept.contains(column))
        .collect()
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let visible = app.visible_indices();
    let title = format!(
        " Hosts - {} shown / {} total ",
        visible.len(),
        app.catalog.hosts().len()
    );
    let block = panel(&title);
    if visible.is_empty() {
        let message = if app.filter.trim().is_empty() {
            "No concrete Host aliases found in this SSH config."
        } else {
            "Nothing matches the current search. Press Esc to clear it."
        };
        frame.render_widget(
            Paragraph::new(message)
                .style(Style::default().fg(theme::MUTED))
                .block(block),
            area,
        );
        return;
    }

    let columns = visible_columns(area.width);
    let header = Row::new(
        columns
            .iter()
            .map(|column| column.header())
            .collect::<Vec<_>>(),
    )
    .style(
        Style::default()
            .fg(theme::MUTED)
            .add_modifier(Modifier::BOLD),
    );
    let rows = visible
        .iter()
        .filter_map(|index| app.catalog.hosts().get(*index))
        .map(|host| row(host, &columns));
    let table = Table::new(
        rows,
        columns
            .iter()
            .map(|column| column.constraint())
            .collect::<Vec<_>>(),
    )
    .header(header)
    .block(block)
    .row_highlight_style(
        Style::default()
            .bg(theme::SELECT_BG)
            .add_modifier(Modifier::BOLD),
    )
    .column_spacing(1);
    let mut state = TableState::default();
    state.select(Some(app.selected.min(visible.len().saturating_sub(1))));
    frame.render_stateful_widget(table, area, &mut state);
}

fn row(host: &Host, columns: &[Column]) -> Row<'static> {
    let cells: Vec<Cell> = columns
        .iter()
        .map(|column| match column {
            Column::Marker => Cell::from("›"),
            Column::Alias => {
                Cell::from(host.alias.clone()).style(Style::default().fg(theme::IDENT))
            }
            Column::Hostname => {
                Cell::from(host.hints.hostname.clone().unwrap_or_else(|| "-".into()))
            }
            Column::User => Cell::from(host.hints.user.clone().unwrap_or_else(|| "-".into())),
            Column::Port => Cell::from(host.hints.port.clone().unwrap_or_else(|| "-".into())),
            Column::Source => Cell::from(source_label(host)),
        })
        .collect();
    Row::new(cells).style(Style::default().fg(theme::TEXT))
}

fn source_label(host: &Host) -> String {
    let file = host
        .source
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| host.source.to_string_lossy());
    format!("{file}:{}", host.line)
}

fn panel(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::ACCENT))
        .title(Span::styled(
            title.to_string(),
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_layout_keeps_only_marker_and_alias() {
        assert_eq!(visible_columns(24), [Column::Marker, Column::Alias]);
    }

    #[test]
    fn wider_layout_adds_columns_in_value_order() {
        let columns = visible_columns(80);
        assert!(columns.contains(&Column::Hostname));
        assert!(columns.contains(&Column::User));
        assert!(!columns.contains(&Column::Source));
        assert_eq!(columns[0], Column::Marker);
        assert_eq!(columns[1], Column::Alias);
    }
}
