mod list;
mod panels;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};

use crate::app::App;

pub fn render(frame: &mut Frame, app: &App) {
    let [left, main] =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)])
            .areas(frame.area());
    let areas: [Rect; 5] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Fill(1),
        Constraint::Fill(1),
        Constraint::Fill(1),
    ])
    .areas(left);
    panels::render_left(frame, areas, app);
    panels::render_main(frame, main, app);
}
