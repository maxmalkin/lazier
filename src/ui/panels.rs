use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use super::list;
use crate::app::{App, PANELS};
use crate::git::{CommitEntry, FileEntry};

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

// Staged entries are green, work-tree entries red, like lazygit.
fn file_line(f: &FileEntry) -> Line<'static> {
    let path_color = if f.staged { Color::Green } else { Color::Red };
    let stage = if f.staged {
        Span::styled("S", Style::new().fg(Color::Green).add_modifier(Modifier::BOLD))
    } else {
        Span::raw(" ")
    };
    Line::from(vec![
        stage,
        Span::styled(f.mark.to_string(), Style::new().fg(mark_color(f.mark))),
        Span::styled(format!(" {}", f.path), Style::new().fg(path_color)),
    ])
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

fn commit_line(c: &CommitEntry, zoomed: bool) -> Line<'static> {
    let mut spans = graph_spans(&c.graph);
    spans.push(Span::raw(" "));
    spans.push(Span::styled(c.id_str().to_string(), Style::new().fg(Color::DarkGray)));
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

pub fn render_left(frame: &mut Frame, areas: [Rect; 5], app: &App) {
    let repo = &app.repo;
    let head = repo.head.clone().unwrap_or_else(|| "(no repo)".into());
    let rows: [&dyn Fn(usize) -> Line<'static>; 5] = [
        &|_| Line::styled(head.clone(), Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)),
        &|i| file_line(&repo.files[i]),
        &|i| {
            let name = &repo.branches[i];
            if Some(name) == repo.head.as_ref() {
                Line::styled(format!("* {name}"), Style::new().fg(Color::Green).add_modifier(Modifier::BOLD))
            } else {
                Line::from(format!("  {name}"))
            }
        },
        &|i| commit_line(&repo.commits[i], false),
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
    for (i, area) in areas.into_iter().enumerate() {
        list::render(frame, area, PANELS[i], app.focus == i, app.selected[i], app.panel_len(i), rows[i]);
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
        &|i| commit_line(&repo.commits[i], true),
    );
}

pub fn render_main(frame: &mut Frame, area: Rect, app: &App) {
    // In hunk mode, show only the hunk under the cursor.
    let (title, text): (String, &str) = match &app.mode {
        crate::app::Mode::Hunks { path, hunks, cursor, .. } => {
            (format!("Stage hunks — {path}"), hunks[*cursor].as_str())
        }
        _ => ("Diff — J/K: scroll".into(), app.repo.diff.as_str()),
    };
    let lines: Vec<Line> = text
        .lines()
        .skip(app.diff_scroll as usize)
        .take(area.height as usize)
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
    frame.render_widget(Paragraph::new(lines).block(Block::bordered().title(title)), area);
}
