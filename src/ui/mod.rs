mod list;
mod panels;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;

use crate::app::{App, Mode};

pub fn render(frame: &mut Frame, app: &App) {
    let [body, bar] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());
    // The zoomed graph view takes the whole body.
    if app.zoom {
        panels::render_zoom(frame, body, app);
        render_bar(frame, bar, app);
        return;
    }
    let [left, main] =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)]).areas(body);
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
    render_bar(frame, bar, app);
}

// The bottom bar shows the active prompt or the last message.
fn render_bar(frame: &mut Frame, area: Rect, app: &App) {
    let line = match &app.mode {
        Mode::Input { prompt, buffer, .. } => {
            Line::styled(format!("{prompt}: {buffer}▏"), Style::new().fg(Color::Cyan))
        }
        Mode::Confirm { prompt, .. } => Line::styled(prompt.clone(), Style::new().fg(Color::Yellow)),
        Mode::Hunks { cursor, hunks, .. } => Line::styled(
            format!("hunk {}/{} — space: stage, j/k: move, esc: back", cursor + 1, hunks.len()),
            Style::new().fg(Color::Magenta),
        ),
        // A failed command shows its message in red.
        Mode::Normal => {
            let color = if app.message_ok { Color::Green } else { Color::Red };
            Line::styled(app.message.clone(), Style::new().fg(color))
        }
    };
    frame.render_widget(line, area);
}
