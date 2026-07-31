mod list;
mod panels;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::app::{App, Mode};

const KEY: Style = Style::new().fg(Color::Yellow);
const DESC: Style = Style::new().fg(Color::Gray);
const DIM: Style = Style::new().fg(Color::DarkGray);

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
type Hints = &'static [(&'static str, &'static str)];
const HINTS: [Hints; 6] = [
    &[("r", "refresh")],
    &[
        ("space", "stage"),
        ("a", "all"),
        ("c", "commit"),
        ("C", "editor"),
        ("s", "stash"),
        ("enter", "hunks/fold"),
        ("o/t", "ours/theirs"),
    ],
    &[
        ("enter", "checkout"),
        ("n", "new"),
        ("d", "delete"),
        ("P", "push"),
        ("p", "pull"),
        ("f", "fetch"),
    ],
    &[("enter", "zoom"), ("i", "rebase"), ("g/G", "top/bottom"), ("ctrl-d/u", "page")],
    &[("enter/a", "apply"), ("p", "pop"), ("d", "drop")],
    &[("j/k", "scroll"), ("g/G", "top/bottom")],
];
const GLOBAL_HINTS: Hints = &[("?", "help"), ("@", "log"), ("q", "quit")];

// Make one line of "key desc · key desc" with the keys in a bright color.
fn hint_line(groups: &[Hints]) -> Line<'static> {
    let mut spans = Vec::new();
    for (gi, group) in groups.iter().enumerate() {
        for (i, (key, desc)) in group.iter().enumerate() {
            if gi > 0 || i > 0 {
                spans.push(Span::styled(" · ", DIM));
            }
            spans.push(Span::styled(*key, KEY));
            spans.push(Span::styled(format!(" {desc}"), DESC));
        }
    }
    Line::from(spans)
}

fn render_bar(frame: &mut Frame, area: Rect, app: &App) {
    let line = match &app.mode {
        Mode::Input { prompt, buffer, .. } => Line::from(vec![
            Span::styled(format!("{prompt}: "), Style::new().fg(Color::Cyan)),
            Span::styled(format!("{buffer}▏"), Style::new().fg(Color::White)),
        ]),
        Mode::Confirm { prompt, .. } => Line::styled(prompt.clone(), Style::new().fg(Color::Yellow)),
        Mode::Hunks { cursor, hunks, .. } => {
            let mut spans = vec![Span::styled(
                format!("hunk {}/{}  ", cursor + 1, hunks.len()),
                Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            )];
            spans.extend(
                hint_line(&[&[("space", "stage"), ("j/k", "move"), ("esc", "back")]]).spans,
            );
            Line::from(spans)
        }
        Mode::Rebase { .. } => hint_line(&[&[
            ("p", "pick"),
            ("r", "reword"),
            ("e", "edit"),
            ("s", "squash"),
            ("f", "fixup"),
            ("d", "drop"),
            ("J/K", "move"),
            ("enter", "run"),
            ("esc", "cancel"),
        ]]),
        Mode::Help => Line::styled("press any key to close the help", Style::new().fg(Color::Cyan)),
        // A stopped rebase takes over three keys.
        Mode::Normal if app.rebase.is_some() => {
            let r = app.rebase.as_ref().unwrap();
            let mut spans = vec![Span::styled(
                format!("REBASE {}/{}  ", r.step, r.total),
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )];
            spans.extend(
                hint_line(&[&[("c", "continue"), ("s", "skip"), ("A", "abort")]]).spans,
            );
            Line::from(spans)
        }
        // A failed command shows its message in red.
        Mode::Normal if !app.message.is_empty() => {
            let color = if app.message_ok { Color::Green } else { Color::Red };
            Line::styled(app.message.clone(), Style::new().fg(color))
        }
        Mode::Normal => hint_line(&[HINTS[app.focus.min(5)], GLOBAL_HINTS]),
    };
    frame.render_widget(line, area);
}

// Each section has a title and its key rows.
const HELP: &[(&str, Hints)] = &[
    (
        "Global",
        &[
            ("tab / shift-tab", "cycle the panes"),
            ("1 2 3 4 5", "go to a panel"),
            ("0", "go to the diff pane"),
            ("j / k", "move the selection"),
            ("ctrl-d / ctrl-u", "move one page"),
            ("g / G", "go to the top or the end"),
            ("J / K", "scroll the diff pane"),
            ("r", "read the repository again"),
            ("? / @ / q", "help, command log, quit"),
        ],
    ),
    (
        "Files [2]",
        &[
            ("space", "stage or unstage the file or the directory"),
            ("a", "stage all files"),
            ("enter", "open the hunks, or fold the directory"),
            ("c / C", "commit here, or commit in the editor"),
            ("s", "put the changes in a stash"),
            ("o / t", "take ours or theirs in a conflict"),
        ],
    ),
    (
        "Branches [3]",
        &[
            ("enter", "check out the branch"),
            ("n / d", "make a new branch, or delete this one"),
            ("P / p / f", "push, pull, or fetch"),
        ],
    ),
    (
        "Commits [4]",
        &[
            ("enter", "open the full graph view"),
            ("i", "start an interactive rebase here"),
            ("↑", "this commit is not on the upstream branch"),
        ],
    ),
    ("Stash [5]", &[("enter / a", "apply the stash"), ("p", "pop it"), ("d", "drop it")]),
    (
        "Rebase",
        &[
            ("p r e s f d", "pick, reword, edit, squash, fixup, drop"),
            ("J / K", "move the commit in the list"),
            ("enter / esc", "run the rebase, or cancel it"),
            ("c / s / A", "while stopped: continue, skip, abort"),
        ],
    ),
];

fn render_help(frame: &mut Frame, body: Rect) {
    let w = 64.min(body.width);
    let inner = w.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();
    for (i, (title, rows)) in HELP.iter().enumerate() {
        if i > 0 {
            lines.push(Line::styled("─".repeat(inner), DIM));
        }
        lines.push(Line::styled(
            format!(" {title}"),
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
        for (key, desc) in rows.iter() {
            lines.push(Line::from(vec![
                Span::styled(format!("  {key:>15}  "), KEY),
                Span::styled(*desc, DESC),
            ]));
        }
    }
    let h = (lines.len() as u16 + 2).min(body.height);
    let area = Rect {
        x: body.x + (body.width.saturating_sub(w)) / 2,
        y: body.y + (body.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(Span::styled(" Keys ", Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)))
                .border_style(Style::new().fg(Color::Cyan)),
        ),
        area,
    );
}
