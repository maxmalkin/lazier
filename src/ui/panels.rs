use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

use super::list;
use crate::app::{App, PANELS};
use crate::git::CommitEntry;
use crate::git::rebase::{TodoAction, TodoItem};

fn mark_color(mark: char) -> Color {
    match mark {
        'A' => Color::Green,
        'M' => Color::Yellow,
        'D' => Color::Red,
        'R' | 'C' => Color::Magenta,
        'T' => Color::Cyan,
        'U' => Color::LightRed,
        _ => Color::DarkGray,
    }
}


// One color for each graph lane. The palette repeats after six lanes.
const LANE_COLORS: [Color; 6] =
    [Color::Cyan, Color::Magenta, Color::Green, Color::Yellow, Color::Blue, Color::Red];

fn graph_spans(graph: &str) -> Vec<Span<'static>> {
    graph
        .chars()
        .enumerate()
        .map(|(i, ch)| {
            Span::styled(ch.to_string(), Style::new().fg(LANE_COLORS[(i / 2) % LANE_COLORS.len()]))
        })
        .collect()
}

fn commit_line(c: &CommitEntry, zoomed: bool, unpushed: bool) -> Line<'static> {
    let mut spans = graph_spans(&c.graph);
    // An up arrow marks a commit that the upstream branch does not have.
    if unpushed {
        spans.push(Span::styled(" ↑", Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
    } else {
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(format!(" {}", c.id_str()), Style::new().fg(Color::DarkGray)));
    if zoomed {
        spans.push(Span::styled(format!(" {}", ymd(c.time)), Style::new().fg(Color::DarkGray)));
        spans.push(Span::styled(format!(" {:<16.16}", &*c.author), Style::new().fg(Color::Blue)));
    }
    spans.push(Span::raw(format!(" {}", c.subject)));
    Line::from(spans)
}

// Convert epoch seconds to a calendar date. This uses the civil-from-days
// algorithm. It saves a date dependency.
fn ymd(secs: u32) -> String {
    let z = (secs / 86400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

// A tree row: a folded or open directory, or a file with its mark.
fn tree_line(app: &App, i: usize) -> Line<'static> {
    let row = &app.tree[i];
    let pad = "  ".repeat(row.depth as usize);
    if let Some(dir) = &row.dir {
        let arrow = if app.collapsed.contains(dir) { '▸' } else { '▾' };
        return Line::styled(
            format!("  {pad}{arrow} {}/", row.name),
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        );
    }
    let f = &app.repo.files[row.file.unwrap_or(0)];
    let path_color = if f.staged { Color::Green } else { Color::Red };
    let stage = if f.staged {
        Span::styled("S", Style::new().fg(Color::Green).add_modifier(Modifier::BOLD))
    } else {
        Span::raw(" ")
    };
    Line::from(vec![
        stage,
        Span::styled(f.mark.to_string(), Style::new().fg(mark_color(f.mark))),
        Span::styled(format!(" {pad}{}", row.name), Style::new().fg(path_color)),
    ])
}

pub fn render_left(frame: &mut Frame, areas: [Rect; 5], app: &App) {
    let repo = &app.repo;
    let mut head = repo.head.clone().unwrap_or_else(|| "(no repo)".into());
    // Show how far the branch is from its upstream.
    if repo.ahead > 0 {
        head.push_str(&format!(" ↑{}", repo.ahead));
    }
    if repo.behind > 0 {
        head.push_str(&format!(" ↓{}", repo.behind));
    }
    // A stopped rebase replaces the branch name with its progress.
    let banner = app
        .rebase
        .as_ref()
        .map(|r| (format!("REBASE {}/{}", r.step, r.total), Color::Yellow));
    let rows: [&dyn Fn(usize) -> Line<'static>; 5] = [
        &|_| match &banner {
            Some((text, color)) => {
                Line::styled(text.clone(), Style::new().fg(*color).add_modifier(Modifier::BOLD))
            }
            None => Line::styled(
                head.clone(),
                Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
        },
        &|i| tree_line(app, i),
        &|i| {
            let name = &repo.branches[i];
            if Some(name) == repo.head.as_ref() {
                Line::styled(format!("* {name}"), Style::new().fg(Color::Green).add_modifier(Modifier::BOLD))
            } else {
                Line::from(format!("  {name}"))
            }
        },
        &|i| {
            let c = &repo.commits[i];
            commit_line(c, false, repo.unpushed.contains(c.id_str()))
        },
        &|i| {
            // Color the stash name before the colon.
            let s = &repo.stashes[i];
            match s.split_once(':') {
                Some((name, rest)) => Line::from(vec![
                    Span::styled(name.to_string(), Style::new().fg(Color::Magenta)),
                    Span::raw(format!(":{rest}")),
                ]),
                None => Line::from(s.clone()),
            }
        },
    ];
    // Each title carries the key that focuses the panel.
    for (i, area) in areas.into_iter().enumerate() {
        let title = format!("[{}] {}", i + 1, PANELS[i]);
        list::render(frame, area, &title, app.focus == i, app.selected[i], app.panel_len(i), rows[i]);
    }
}

/// The zoomed graph view fills the whole body. It shows the graph with the
/// date and the author of each commit.
pub fn render_zoom(frame: &mut Frame, area: Rect, app: &App) {
    let repo = &app.repo;
    list::render(
        frame,
        area,
        "Commits — enter: back, j/k, ctrl-d/u, g/G",
        true,
        app.selected[3],
        app.panel_len(3),
        &|i| {
            let c = &repo.commits[i];
            commit_line(c, true, repo.unpushed.contains(c.id_str()))
        },
    );
}

/// The todo list editor of an interactive rebase. The list shows the newest
/// commit first, the same order as the commits panel.
pub fn render_rebase(frame: &mut Frame, area: Rect, items: &[TodoItem], cursor: usize) {
    list::render(
        frame,
        area,
        "Interactive rebase — enter: run, esc: cancel",
        true,
        cursor,
        items.len(),
        &|i| {
            let it = &items[i];
            let color = match it.action {
                TodoAction::Pick => Color::Green,
                TodoAction::Reword => Color::Cyan,
                TodoAction::Edit => Color::Yellow,
                TodoAction::Squash | TodoAction::Fixup => Color::Magenta,
                TodoAction::Drop => Color::Red,
            };
            Line::from(vec![
                Span::styled(format!("{:<7}", it.action.word()), Style::new().fg(color).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{} ", it.id), Style::new().fg(Color::DarkGray)),
                Span::raw(it.subject.clone()),
            ])
        },
    );
}

pub fn render_main(frame: &mut Frame, area: Rect, app: &App) {
    // The rebase editor and the hunk view replace the diff pane.
    if let crate::app::Mode::Rebase { items, cursor, .. } = &app.mode {
        render_rebase(frame, area, items, *cursor);
        return;
    }
    // In hunk mode, show only the hunk under the cursor.
    let (title, text): (String, &str) = match &app.mode {
        crate::app::Mode::Hunks { path, hunks, cursor, .. } => {
            (format!("Stage hunks — {path}"), hunks[*cursor].as_str())
        }
        _ => ("[0] Diff".into(), app.repo.diff.as_str()),
    };
    let focused = app.focus == 5;
    // A wrapped line can use more than one row. Take more lines than the
    // height, then let the widget cut what does not fit.
    let lines: Vec<Line> = text
        .lines()
        .skip(app.diff_scroll as usize)
        .take(area.height as usize * 4)
        .map(|l| {
            // File header lines come before the +/- check. A "---" line is
            // a header, not a removal.
            let style = if l.starts_with("diff ") || l.starts_with("index ") || l.starts_with("--- ") || l.starts_with("+++ ") {
                Style::new().add_modifier(Modifier::BOLD)
            } else if l.starts_with("commit ") || l.starts_with("Author") || l.starts_with("Date") || l.starts_with("Merge") {
                Style::new().fg(Color::Yellow)
            } else {
                match l.as_bytes().first() {
                    Some(b'+') => Style::new().fg(Color::Green),
                    Some(b'-') => Style::new().fg(Color::Red),
                    Some(b'@') => Style::new().fg(Color::Cyan),
                    _ => Style::new(),
                }
            };
            Line::from(l.to_string()).style(style)
        })
        .collect();
    let border = if focused { Color::Green } else { Color::DarkGray };
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(title).border_style(Style::new().fg(border))),
        area,
    );
}

/// The command log shows the last git commands and their results.
pub fn render_log(frame: &mut Frame, area: Rect, app: &App) {
    let take = area.height.saturating_sub(2) as usize;
    let start = app.cmd_log.len().saturating_sub(take);
    let lines: Vec<Line> = app.cmd_log[start..]
        .iter()
        .map(|(ok, line)| {
            let color = if *ok { Color::DarkGray } else { Color::Red };
            Line::styled(line.clone(), Style::new().fg(color))
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().title("[@] Command log").border_style(Style::new().fg(Color::DarkGray))),
        area,
    );
}
