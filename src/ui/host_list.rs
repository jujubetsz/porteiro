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

// Powerline semicircles occupy one terminal cell each in a Nerd Font.
const TAG_LEFT: &str = "\u{e0b6}";
const TAG_RIGHT: &str = "\u{e0b4}";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Marker,
    Alias,
    Tags,
    Hostname,
    User,
    Port,
    Source,
}

impl Column {
    const DISPLAY_ORDER: [Self; 7] = [
        Self::Marker,
        Self::Alias,
        Self::Tags,
        Self::Hostname,
        Self::User,
        Self::Port,
        Self::Source,
    ];
    const OPTIONAL_BY_VALUE: [Self; 5] = [
        Self::Tags,
        Self::Hostname,
        Self::User,
        Self::Port,
        Self::Source,
    ];

    fn header(self) -> &'static str {
        match self {
            Self::Marker => "",
            Self::Alias => "Alias",
            Self::Tags => "Tags",
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
            Self::Tags => 20,
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

fn visible_columns(area_width: u16, has_tags: bool) -> Vec<Column> {
    let mut kept = vec![Column::Marker, Column::Alias];
    let cost = |columns: &[Column]| -> u16 {
        columns.iter().map(|column| column.width()).sum::<u16>()
            + columns.len().saturating_sub(1) as u16
            + 2
    };
    for candidate in Column::OPTIONAL_BY_VALUE {
        if candidate == Column::Tags && !has_tags {
            continue;
        }
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

    // Keep the layout stable while searching or opening groups. Only a catalog
    // reload can change whether the optional Tags column is useful at all.
    let has_tags = app
        .catalog
        .hosts()
        .iter()
        .any(|host| !host.metadata.tags.is_empty());
    let columns = visible_columns(area.width, has_tags);
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
                    app.list.prefix(position),
                ),
                VisibleRow::Group(index) => group_row(
                    &app.list.groups[index],
                    app.list.is_expanded(index),
                    &columns,
                    is_selected,
                    usize::from(alias_width),
                    app.list.prefix(position),
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
    prefix: &str,
) -> Row<'static> {
    let marker = if expanded { "▾" } else { "▸" };
    let count = if group.matched == group.total {
        group.total.to_string()
    } else {
        format!("{}/{}", group.matched, group.total)
    };
    let prefix = fit_tree_prefix(prefix, width.saturating_sub(12));
    let label_width = width.saturating_sub(Span::raw(&prefix).width());
    let name = truncate_label(&group.name, label_width.saturating_sub(count.len() + 5));
    let label = truncate_label(&format!("{marker} {name} ({count})"), label_width);
    let cells = columns
        .iter()
        .map(|column| match column {
            Column::Marker => Cell::from(if selected { "›" } else { " " }),
            Column::Alias => Cell::from(Line::from(vec![
                Span::styled(prefix.clone(), Style::default().fg(theme::MUTED)),
                Span::styled(
                    label.clone(),
                    Style::default()
                        .fg(theme::accent(group.color))
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
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
    prefix: &str,
) -> Row<'static> {
    let cells: Vec<Cell> = columns
        .iter()
        .map(|column| match column {
            Column::Marker => Cell::from(if selected { "›" } else { " " }),
            Column::Alias => Cell::from(alias_line(host, badge, alias_width, prefix)),
            Column::Tags => Cell::from(tags_line(
                &host.metadata.tags,
                usize::from(Column::Tags.width()),
            )),
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

fn alias_line(host: &Host, badge: Option<&str>, width: usize, prefix: &str) -> Line<'static> {
    // Reserve the connection flag before a long alias. Below
    // the minimum useful width, the status line still reports the pending flag.
    let badge = badge.filter(|flag| width >= flag.len() + 6);
    let alias_width = width.saturating_sub(badge.map_or(0, |flag| flag.len() + 4));
    let prefix = fit_tree_prefix(prefix, alias_width.saturating_sub(12));
    let prefix_width = Span::raw(&prefix).width();
    let mut spans = vec![
        Span::styled(prefix, Style::default().fg(theme::MUTED)),
        Span::styled(
            truncate_label(&host.alias, alias_width.saturating_sub(prefix_width)),
            Style::default().fg(theme::accent(host.metadata.color)),
        ),
    ];
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
    Line::from(spans)
}

/// Render whole capsules in their own column, reserving a count for hidden
/// tags. Measure terminal cells so wide and combining graphemes stay intact.
fn tags_line(tags: &[String], width: usize) -> Line<'static> {
    let mut spans = Vec::new();
    let mut used = 0;
    let caps = Style::default()
        .fg(theme::TAG_BG)
        .remove_modifier(Modifier::BOLD);
    let label = Style::default()
        .fg(theme::TAG_FG)
        .bg(theme::TAG_BG)
        .remove_modifier(Modifier::BOLD);
    for (index, tag) in tags.iter().enumerate() {
        let gap = usize::from(used > 0);
        let label_width = gap + 2 + Span::raw(tag).width();
        let remaining = tags.len() - index - 1;
        let suffix = if remaining > 0 {
            format!(" +{remaining}")
        } else {
            String::new()
        };
        if used + label_width + suffix.len() > width {
            let overflow = format!("{}+{}", if used > 0 { " " } else { "" }, remaining + 1);
            if used + overflow.len() <= width {
                spans.push(Span::styled(overflow, Style::default().fg(theme::MUTED)));
            }
            break;
        }
        if gap > 0 {
            spans.push(Span::raw(" "));
        }
        // The caps inherit the row background; only their foreground and the
        // label background form the capsule, including on the selected row.
        spans.push(Span::styled(TAG_LEFT, caps));
        spans.push(Span::styled(tag.clone(), label));
        spans.push(Span::styled(TAG_RIGHT, caps));
        used += label_width;
    }
    Line::from(spans)
}

/// Omit distant ancestors before sacrificing the destination label. Every
/// connector occupies three cells and contains no user-authored text.
fn fit_tree_prefix(prefix: &str, width: usize) -> String {
    let cells = prefix.chars().count();
    if cells <= width {
        return prefix.to_string();
    }
    if width < 5 {
        return String::new();
    }
    let tail = ((width - 2) / 3) * 3;
    format!(
        "… {}",
        prefix.chars().skip(cells - tail).collect::<String>()
    )
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
        for has_tags in [false, true] {
            assert_eq!(
                visible_columns(24, has_tags),
                [Column::Marker, Column::Alias]
            );
        }
    }

    #[test]
    fn wider_layout_adds_columns_in_value_order() {
        let columns = visible_columns(80, false);
        assert!(columns.contains(&Column::Hostname));
        assert!(columns.contains(&Column::User));
        assert!(!columns.contains(&Column::Source));
        assert!(!columns.contains(&Column::Tags));
        assert_eq!(columns[0], Column::Marker);
        assert_eq!(columns[1], Column::Alias);
    }

    #[test]
    fn tags_have_their_own_column_only_when_used_and_space_permits() {
        assert_eq!(
            visible_columns(74, true),
            [
                Column::Marker,
                Column::Alias,
                Column::Tags,
                Column::Hostname
            ]
        );
        assert!(!visible_columns(44, true).contains(&Column::Tags));
        assert!(visible_columns(45, true).contains(&Column::Tags));
        assert!(!visible_columns(200, false).contains(&Column::Tags));
    }

    #[test]
    fn long_aliases_leave_space_for_the_connection_flag() {
        let host = Host {
            alias: "a-very-long-production-alias".into(),
            source: "config".into(),
            line: 1,
            hints: Default::default(),
            metadata: crate::catalog::HostMetadata {
                group: vec!["Work".into()],
                tags: vec!["production".into()],
                color: None,
                hidden: false,
            },
        };
        let line = alias_line(&host, Some("-vvv"), 20, "└─ ");
        assert!(line.to_string().contains('…'));
        assert!(line.to_string().ends_with(" -vvv "));
        assert!(!line.to_string().contains("production"));
        assert_eq!(line.width(), 20);
        assert!(alias_line(&host, Some("-vvv"), 3, "└─ ").width() <= 3);
    }

    #[test]
    fn tag_badges_fit_unicode_cells_and_report_overflow_without_cutting_tags() {
        let tags = vec!["東京".into(), "e\u{301}".into()];
        let full = tags_line(&tags, 10);
        assert_eq!(full.to_string(), "東京 e\u{301}");
        assert_eq!(full.width(), 10);
        let partial = tags_line(&tags, 9);
        assert_eq!(partial.to_string(), "東京 +1");
        assert_eq!(partial.width(), 9);
        assert_eq!(tags_line(&tags, 8).to_string(), "+2");
        assert_eq!(tags_line(&[], 20).to_string(), "");
        for width in 0..30 {
            assert!(tags_line(&tags, width).width() <= width);
        }
        let long_tags = vec!["a-tag-too-long-for-this-column".into(); 12];
        assert_eq!(tags_line(&long_tags, 20).to_string(), "+12");
        assert_eq!(tags_line(&long_tags, 2).to_string(), "");
    }

    #[test]
    fn truncation_respects_wide_characters_and_combining_graphemes() {
        assert_eq!(truncate_label("東京", 4), "東京");
        assert_eq!(truncate_label("東京", 3), "東…");
        assert_eq!(truncate_label("e\u{301}xyz", 3), "e\u{301}x…");
        assert_eq!(truncate_label("東京", 0), "");
    }
}
