use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use super::list;
use crate::app::{App, PANELS};

pub fn render_left(frame: &mut Frame, areas: [Rect; 5], app: &App) {
    let repo = &app.repo;
    let head = repo.head.clone().unwrap_or_else(|| "(no repo)".into());
    let rows: [&dyn Fn(usize) -> Line<'static>; 5] = [
        &|_| Line::from(head.clone()),
        &|i| {
            let f = &repo.files[i];
            let stage = if f.staged { 'S' } else { ' ' };
            Line::from(format!("{stage}{} {}", f.mark, f.path))
        },
        &|i| {
            let name = &repo.branches[i];
            let cur = if Some(name) == repo.head.as_ref() { '*' } else { ' ' };
            Line::from(format!("{cur} {name}"))
        },
        &|i| {
            let c = &repo.commits[i];
            Line::from(format!("{} {}", c.id_str(), c.subject))
        },
        &|i| Line::from(repo.stashes[i].clone()),
    ];
    for (i, area) in areas.into_iter().enumerate() {
        list::render(frame, area, PANELS[i], app.focus == i, app.selected[i], app.panel_len(i), rows[i]);
    }
}

pub fn render_main(frame: &mut Frame, area: Rect, app: &App) {
    // ponytail: there is no scroll in the diff view. Add scroll in phase 3.
    let lines: Vec<Line> = app
        .repo
        .diff
        .lines()
        .take(area.height as usize)
        .map(|l| {
            let style = match l.as_bytes().first() {
                Some(b'+') => Style::new().green(),
                Some(b'-') => Style::new().red(),
                Some(b'@') => Style::new().cyan(),
                _ => Style::new(),
            };
            Line::from(l.to_string()).style(style)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(Block::bordered().title("Diff")), area);
}
