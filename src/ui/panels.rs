use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use super::list;
use crate::app::{App, PANELS};

// ponytail: fake data until phase 2 wires gix
const FILES: &[&str] = &["M  src/main.rs", "A  src/app.rs", "M  Cargo.toml", "?? notes.md"];
const BRANCHES: &[&str] = &["* main", "  feature/ui", "  fix/pty-size"];
const STASH: &[&str] = &["stash@{0}: WIP on main", "stash@{1}: pty experiment"];
const FAKE_COMMITS: usize = 100_000; // deep enough to prove virtualization

pub fn panel_len(panel: usize) -> usize {
    match panel {
        0 => 1,
        1 => FILES.len(),
        2 => BRANCHES.len(),
        3 => FAKE_COMMITS,
        4 => STASH.len(),
        _ => 0,
    }
}

fn commit_row(i: usize) -> Line<'static> {
    Line::from(format!("{:07x} fake: commit subject #{i}", 0xa0c000 + i))
}

pub fn render_left(frame: &mut Frame, areas: [Rect; 5], app: &App) {
    let rows: [&dyn Fn(usize) -> Line<'static>; 5] = [
        &|_| Line::from("main → origin/main"),
        &|i| Line::from(FILES[i]),
        &|i| Line::from(BRANCHES[i]),
        &commit_row,
        &|i| Line::from(STASH[i]),
    ];
    for (i, area) in areas.into_iter().enumerate() {
        list::render(frame, area, PANELS[i], app.focus == i, app.selected[i], panel_len(i), rows[i]);
    }
}

pub fn render_main(frame: &mut Frame, area: Rect, app: &App) {
    // ponytail: placeholder proving focus/selection plumbing; becomes the diff view in phase 2
    let text = format!("selected: {} #{}", PANELS[app.focus], app.selected[app.focus]);
    frame.render_widget(Paragraph::new(text).block(Block::bordered().title("Diff")), area);
}
