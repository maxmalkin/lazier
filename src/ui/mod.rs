mod list;
mod panels;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};

use crate::app::{App, CommitPurpose, Mode};

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
    match &app.mode {
        Mode::Help => render_help(frame, body),
        Mode::Worktrees { list, cursor } => {
            let h = (list.len() as u16 + 2).max(4);
            panels::render_worktrees(frame, centered(body, 78, h), list, *cursor);
        }
        Mode::CommitMsg { summary, body: text, on_body, purpose } => {
            render_commit(frame, body, summary, text, *on_body, purpose)
        }
        _ => {}
    }
}

// Put a box of the given size in the middle of an area.
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

/// The commit message window. The summary line is on top. The body is
/// below it. Tab moves between them.
fn render_commit(
    frame: &mut Frame,
    body_area: Rect,
    summary: &str,
    body: &str,
    on_body: bool,
    purpose: &CommitPurpose,
) {
    let area = centered(body_area, 72, 14);
    frame.render_widget(Clear, area);
    let title = match purpose {
        CommitPurpose::New => " Commit ",
        CommitPurpose::Reword(0) => " Reword HEAD ",
        CommitPurpose::Reword(_) => " Reword (runs a rebase) ",
    };
    let outer = Block::bordered()
        .title(Span::styled(title, Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)))
        .border_style(Style::new().fg(Color::Cyan));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let [top, mid, hint] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    let active = Style::new().fg(Color::Green);
    let idle = Style::new().fg(Color::DarkGray);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(summary.to_string(), Style::new().fg(Color::White)),
            Span::styled(if on_body { "" } else { "▏" }, active),
        ]))
        .block(
            Block::bordered()
                .title("summary")
                .border_style(if on_body { idle } else { active }),
        ),
        top,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(body.to_string(), Style::new().fg(Color::White)),
            Span::styled(if on_body { "▏" } else { "" }, active),
        ]))
        .wrap(Wrap { trim: false })
        .block(
            Block::bordered()
                .title("body (optional)")
                .border_style(if on_body { active } else { idle }),
        ),
        mid,
    );
    let keys: Hints = match (on_body, purpose) {
        (true, _) => &[("enter", "new line"), ("tab", "summary"), ("esc", "cancel")],
        (false, CommitPurpose::New) => &[("enter", "commit"), ("tab", "body"), ("esc", "cancel")],
        (false, _) => &[("enter", "reword"), ("tab", "body"), ("esc", "cancel")],
    };
    frame.render_widget(hint_line(&[keys]), hint);
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
        ("d/D", "delete"),
        ("R", "rename"),
        ("m", "merge"),
        ("P/p/f", "push/pull/fetch"),
    ],
    &[
        ("enter", "zoom"),
        ("i", "rebase"),
        ("w", "reword"),
        ("v", "revert"),
        ("y", "cherry-pick"),
        ("b/o", "bisect bad/good"),
    ],
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
        Mode::Hunks { picked, .. } => {
            let mut spans = vec![Span::styled(
                format!("{} marked  ", picked.len()),
                Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            )];
            spans.extend(
                hint_line(&[&[
                    ("space", "mark line"),
                    ("enter", "stage marked"),
                    ("a", "stage hunk"),
                    ("J/K", "hunk"),
                    ("esc", "back"),
                ]])
                .spans,
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
        Mode::Worktrees { .. } => hint_line(&[&[
            ("enter", "go to it"),
            ("n", "new"),
            ("d", "remove"),
            ("esc", "close"),
        ]]),
        // A bisect takes over four keys of the commits panel.
        Mode::Normal if app.repo.bisecting => {
            let mut spans = vec![Span::styled(
                "BISECT  ",
                Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            )];
            spans.extend(
                hint_line(&[&[
                    ("b", "bad"),
                    ("o", "good"),
                    ("S", "skip"),
                    ("A", "reset"),
                    ("@", "log"),
                ]])
                .spans,
            );
            Line::from(spans)
        }
        // The commit window draws its own key hints.
        Mode::CommitMsg { .. } => Line::default(),
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
            ("c / C", "open the commit window, or use the editor"),
            ("s", "put the changes in a stash"),
            ("o / t", "take ours or theirs in a conflict"),
        ],
    ),
    (
        "Branches [3]",
        &[
            ("enter", "check out the branch"),
            ("n", "make a new branch"),
            ("d / D", "delete it, or delete it by force"),
            ("R", "give the branch a new name"),
            ("m", "merge the branch into the current one"),
            ("P / p / f", "push, pull, or fetch"),
            ("● ↑ ↓", "current branch, to push, to pull"),
        ],
    ),
    (
        "Hunk view",
        &[
            ("j / k", "move to another line"),
            ("space", "mark the line, or remove the mark"),
            ("enter", "stage the marked lines"),
            ("a", "stage the full hunk"),
            ("J / K", "go to another hunk"),
        ],
    ),
    (
        "Commit window",
        &[
            ("enter", "commit, or make a new line in the body"),
            ("tab", "move between the summary and the body"),
            ("esc", "cancel"),
        ],
    ),
    (
        "Commits [4]",
        &[
            ("enter", "open the full graph view"),
            ("i", "start an interactive rebase here"),
            ("w", "give the commit a new message"),
            ("v", "revert the commit"),
            ("y", "put its changes in the index, with no commit"),
            ("↑", "this commit is not on the upstream branch"),
        ],
    ),
    (
        "Bisect [4]",
        &[
            ("b", "start a bisect here, or mark a bad commit"),
            ("o", "mark a good commit"),
            ("S / A", "skip this commit, or end the bisect"),
        ],
    ),
    (
        "Worktrees",
        &[
            ("W", "open the list"),
            ("enter", "go to that worktree"),
            ("n / d", "make one, or remove one"),
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
