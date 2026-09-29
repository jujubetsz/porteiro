use super::{
    list::{Group, VisibleRow},
    theme, App,
};
use crate::catalog::Host;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table};
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

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let visible = &app.list.rows;
    let title = format!(
        " Hosts - {} shown / {} total ",
        app.list.shown_hosts,
        app.catalog.hosts().len()
    );
    let block = panel(&title);
    if visible.is_empty() {
        let message = if app.filter.trim().is_empty() {
            "No visible Host aliases found in this SSH config."
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
    let selected = app.selected.min(visible.len().saturating_sub(1));
    let badge = app.verbosity_flag();
    let alias_width = area.width.saturating_sub(
        2 + columns.len().saturating_sub(1) as u16
            + columns
                .iter()
                .filter(|column| **column != Column::Alias)
                .map(|column| column.width())
                .sum::<u16>(),
    );
    let rows: Vec<_> = visible
        .iter()
        .enumerate()
        .map(|(position, entry)| {
            let is_selected = position == selected;
            match *entry {
                VisibleRow::Host(index) => row(
                    &app.catalog.hosts()[index],
                    &columns,
                    is_selected,
                    badge.as_deref().filter(|_| is_selected),
                    usize::from(alias_width),
                ),
                VisibleRow::Group(index) => group_row(
                    &app.list.groups[index],
                    app.list.is_expanded(index),
                    &columns,
                    is_selected,
                    usize::from(alias_width),
                ),
            }
        })
        .collect();
    // The selected row is styled here rather than through the table's
    // highlight style, which would paint over the verbosity badge.
    let table = Table::new(
        rows,
        columns
            .iter()
            .map(|column| column.constraint())
            .collect::<Vec<_>>(),
    )
    .header(header)
    .block(block)
    .column_spacing(1);
    // Ratatui updates the offset while rendering. Retaining that state avoids
    // snapping the viewport back to the top on every navigation event.
    app.table_state.select(Some(selected));
    frame.render_stateful_widget(table, area, &mut app.table_state);
}

fn group_row(
    group: &Group,
    expanded: bool,
    columns: &[Column],
    selected: bool,
    width: usize,
) -> Row<'static> {
    let marker = if expanded { "▾" } else { "▸" };
    let count = if group.matches.len() == group.total {
        group.total.to_string()
    } else {
        format!("{}/{}", group.matches.len(), group.total)
    };
    let name = truncate_label(&group.name, width.saturating_sub(count.len() + 5));
    let cells = columns
        .iter()
        .map(|column| match column {
            Column::Marker => Cell::from(if selected { "›" } else { " " }),
            Column::Alias => Cell::from(Span::styled(
                format!("{marker} {name} ({count})"),
                Style::default()
                    .fg(theme::accent(group.color))
                    .add_modifier(Modifier::BOLD),
            )),
            _ => Cell::from(""),
        })
        .collect::<Vec<_>>();
    Row::new(cells).style(row_style(selected))
}

fn row(
    host: &Host,
    columns: &[Column],
    selected: bool,
    badge: Option<&str>,
    alias_width: usize,
) -> Row<'static> {
    let cells: Vec<Cell> = columns
        .iter()
        .map(|column| match column {
            Column::Marker => Cell::from(if selected { "›" } else { " " }),
            Column::Alias => Cell::from(alias_line(host, badge, alias_width)),
            Column::Hostname => {
                Cell::from(host.hints.hostname.clone().unwrap_or_else(|| "-".into()))
            }
            Column::User => Cell::from(host.hints.user.clone().unwrap_or_else(|| "-".into())),
            Column::Port => Cell::from(host.hints.port.clone().unwrap_or_else(|| "-".into())),
            Column::Source => Cell::from(source_label(host)),
        })
        .collect();
    Row::new(cells).style(row_style(selected))
}

fn row_style(selected: bool) -> Style {
    let mut style = Style::default().fg(theme::TEXT);
    if selected {
        style = style.bg(theme::SELECT_BG).add_modifier(Modifier::BOLD);
    }
    style
}

/// Optional tags use only the space left after the alias and connection flag.
/// Measure terminal cells (including wide Unicode), not UTF-8 bytes.
fn alias_line(host: &Host, badge: Option<&str>, width: usize) -> Line<'static> {
    let indent = if host.metadata.group.is_some() {
        "  "
    } else {
        ""
    };
    // Reserve the connection flag before optional tags or a long alias. Below
    // the minimum useful width, the status line still reports the pending flag.
    let badge = badge.filter(|flag| width >= flag.len() + 6);
    let alias_width = width.saturating_sub(badge.map_or(0, |flag| flag.len() + 4));
    let mut spans = vec![Span::styled(
        truncate_label(&format!("{indent}{}", host.alias), alias_width),
        Style::default().fg(theme::accent(host.metadata.color)),
    )];
    if let Some(flag) = badge {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!(" {flag} "),
            Style::default()
                .fg(theme::BG)
                .bg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
    }
    let mut used: usize = spans.iter().map(Span::width).sum();
    for (index, tag) in host.metadata.tags.iter().enumerate() {
        let label = format!(" [{tag}]");
        let label_width = Span::raw(&label).width();
        let remaining = host.metadata.tags.len() - index - 1;
        let suffix = if remaining > 0 {
            format!(" +{remaining}")
        } else {
            String::new()
        };
        if used + label_width + suffix.len() > width {
            let overflow = format!(" +{}", remaining + 1);
            if used + overflow.len() <= width {
                spans.push(Span::styled(overflow, Style::default().fg(theme::MUTED)));
            }
            break;
        }
        spans.push(Span::styled(
            label,
            Style::default().fg(theme::accent(host.metadata.color)),
        ));
        used += label_width;
    }
    Line::from(spans)
}

/// Keep grapheme clusters intact: truncating by bytes or scalar values can cut
/// an accented letter or an emoji sequence in half. Ratatui's own measurement
/// gives the same cell widths that its renderer uses.
fn truncate_label(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let span = Span::raw(value);
    if span.width() <= width {
        return value.to_string();
    }
    let mut result = String::new();
    let mut used = 0;
    for grapheme in span.styled_graphemes(Style::default()) {
        let cells = Span::raw(grapheme.symbol).width();
        if used + cells > width - 1 {
            break;
        }
        result.push_str(grapheme.symbol);
        used += cells;
    }
    result.push('…');
    result
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

    #[test]
    fn long_aliases_leave_space_for_the_connection_flag_before_tags() {
        let host = Host {
            alias: "a-very-long-production-alias".into(),
            source: "config".into(),
            line: 1,
            hints: Default::default(),
            metadata: crate::catalog::HostMetadata {
                group: Some("Work".into()),
                tags: vec!["production".into()],
                color: None,
                hidden: false,
            },
        };
        let line = alias_line(&host, Some("-vvv"), 20);
        assert!(line.to_string().contains('…'));
        assert!(line.to_string().ends_with(" -vvv "));
        assert!(!line.to_string().contains("[production]"));
        assert_eq!(line.width(), 20);
        assert!(alias_line(&host, Some("-vvv"), 3).width() <= 3);
    }

    #[test]
    fn truncation_respects_wide_characters_and_combining_graphemes() {
        assert_eq!(truncate_label("東京", 4), "東京");
        assert_eq!(truncate_label("東京", 3), "東…");
        assert_eq!(truncate_label("e\u{301}xyz", 3), "e\u{301}x…");
        assert_eq!(truncate_label("東京", 0), "");
    }
}
