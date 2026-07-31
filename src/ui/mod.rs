mod list;
mod panels;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Clear, Paragraph};

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
    // The command log takes the lower part of the main column.
    if app.show_log {
        let [diff, log] = Layout::vertical([Constraint::Fill(1), Constraint::Length(8)]).areas(main);
        panels::render_main(frame, diff, app);
        panels::render_log(frame, log, app);
    } else {
        panels::render_main(frame, main, app);
    }
    render_bar(frame, bar, app);
    if matches!(app.mode, Mode::Help) {
        render_help(frame, body);
    }
}

// Key hints for each pane. The bar shows them when there is no message.
const HINTS: [&str; 6] = [
    "r refresh",
    "space stage · a all · c commit · C editor · s stash · enter hunks/fold · o/t conflict",
    "enter checkout · n new · d delete · P push · p pull · f fetch",
    "enter zoom · i rebase · g/G top/bottom · ctrl-d/u page",
    "enter/a apply · p pop · d drop",
    "j/k scroll · g top",
];

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
        Mode::Help => Line::styled("press any key to close the help", Style::new().fg(Color::Cyan)),
        Mode::Rebase { .. } => Line::styled(
            "p pick · r reword · e edit · s squash · f fixup · d drop · J/K move · enter run · esc cancel",
            Style::new().fg(Color::Magenta),
        ),
        // A stopped rebase takes over three keys.
        Mode::Normal if app.rebase.is_some() => {
            let r = app.rebase.as_ref().unwrap();
            Line::styled(
                format!("REBASE {}/{} — c continue · s skip · A abort", r.step, r.total),
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )
        }
        // A failed command shows its message in red.
        Mode::Normal if !app.message.is_empty() => {
            let color = if app.message_ok { Color::Green } else { Color::Red };
            Line::styled(app.message.clone(), Style::new().fg(color))
        }
        Mode::Normal => Line::styled(
            format!("{} · ? help · @ log · q quit", HINTS[app.focus.min(5)]),
            Style::new().fg(Color::DarkGray),
        ),
    };
    frame.render_widget(line, area);
}

const HELP: &str = "\
 Global      tab/shift-tab cycle panes    1-5 panes  0 diff pane
             j/k move      ctrl-d/u page  g/G top/bottom
             J/K scroll diff              r refresh
             ? this help   @ command log  q quit

 Files [2]   space stage/unstage  a stage all  enter hunks or fold dir
             c commit  C commit in editor  s stash  o/t take ours/theirs

 Branches[3] enter checkout  n new branch  d delete
             P push  p pull  f fetch  (these use the real terminal)

 Commits [4] enter zoomed graph view  i interactive rebase from here
             ↑ marks unpushed commits

 Rebase      p pick  r reword  e edit  s squash  f fixup  d drop
             J/K move a commit  enter run  esc cancel
             while stopped: c continue  s skip  A abort

 Stash [5]   enter/a apply  p pop  d drop

 Hunk mode   space stage hunk  j/k move  esc back";

fn render_help(frame: &mut Frame, body: Rect) {
    let w = 76.min(body.width);
    let h = 20.min(body.height);
    let area = Rect {
        x: body.x + (body.width - w) / 2,
        y: body.y + (body.height - h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(HELP)
            .block(Block::bordered().title("Help").border_style(Style::new().fg(Color::Cyan))),
        area,
    );
}
