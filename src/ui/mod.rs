mod list;
mod words;
mod panels;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};

use crate::app::{App, CommitPurpose, Mode};

pub use list::offset as list_offset;

const KEY: Style = Style::new().fg(Color::Yellow);
const DESC: Style = Style::new().fg(Color::Gray);
const DIM: Style = Style::new().fg(Color::DarkGray);

/// Where each panel sits. The renderer and the mouse both need this, thus
/// one function gives it to both.
pub struct Panes {
    pub left: [Rect; 5],
    pub diff: Rect,
    pub log: Option<Rect>,
}

pub fn panes(area: Rect, show_log: bool) -> Panes {
    let [body, _bar] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
    let [left, main] =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)]).areas(body);
    let left: [Rect; 5] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Fill(1),
        Constraint::Fill(1),
        Constraint::Fill(1),
    ])
    .areas(left);
    if show_log {
        // The diff takes most of the height. The log needs only a few rows.
        let [diff, log] = Layout::vertical([Constraint::Fill(1), Constraint::Length(6)]).areas(main);
        Panes { left, diff, log: Some(log) }
    } else {
        Panes { left, diff: main, log: None }
    }
}

pub fn render(frame: &mut Frame, app: &App) {
    let [body, bar] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());
    // The zoomed graph view takes the whole body.
    if app.zoom {
        panels::render_zoom(frame, body, app);
        render_bar(frame, bar, app);
        return;
    }
    let p = panes(frame.area(), app.show_log);
    panels::render_left(frame, p.left, app);
    panels::render_main(frame, p.diff, app);
    if let Some(log) = p.log {
        panels::render_log(frame, log, app);
    }
    render_bar(frame, bar, app);
    match &app.mode {
        Mode::Help => render_help(frame, body),
        Mode::Confirm { prompt, .. } => render_confirm(frame, body, prompt),
        Mode::Worktrees { list, cursor } => {
            let h = (list.len() as u16 + 2).max(4);
            let home = std::env::var("HOME").unwrap_or_default();
            panels::render_worktrees(frame, centered(body, 82, h), list, *cursor, &home);
        }
        Mode::NewWorktree { branch, path, on_path, .. } => {
            render_new_worktree(frame, body, branch, path, *on_path, app)
        }
        Mode::Ignore { pattern, tracked } => render_ignore(frame, body, pattern, *tracked),
        Mode::Stash { path } => render_stash(frame, body, path.as_deref()),
        Mode::Reset { target, subject } => render_reset(frame, body, target, subject),
        Mode::Error { cmd, output } => render_error(frame, body, cmd, output),
        Mode::Reflog { list, cursor } => {
            let h = (list.len() as u16 + 2).min(body.height).max(4);
            panels::render_reflog(frame, centered(body, 80, h), list, *cursor);
        }
        // Blame needs the room, thus it takes the whole body.
        Mode::Blame { path, lines, cursor } => {
            panels::render_blame(frame, body, path, lines, *cursor)
        }
        Mode::Submodules { list, cursor } => {
            let h = (list.len() as u16 + 2).min(body.height).max(4);
            panels::render_submodules(frame, centered(body, 76, h), list, *cursor);
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

/// The window that shows a command that failed. The command log can be
/// closed, thus a failure needs a window of its own.
fn render_error(frame: &mut Frame, body: Rect, cmd: &str, output: &[String]) {
    let w = 72.min(body.width);
    let inner = w.saturating_sub(4).max(1) as usize;
    // A long line wraps, thus count the rows it will use.
    let rows: usize = output.iter().map(|l| l.chars().count().div_ceil(inner).max(1)).sum();
    let h = (rows as u16 + 5).min(body.height);
    let area = centered(body, w, h);
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .title(Span::styled(" Failed ", Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)))
        .border_style(Style::new().fg(Color::Red));
    let text = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = vec![Line::styled(cmd.to_string(), Style::new().fg(Color::White)), Line::default()];
    for l in output {
        lines.push(Line::styled(l.clone(), Style::new().fg(Color::Red)));
    }
    lines.push(Line::default());
    lines.push(Line::styled("press any key to close", DIM));
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        Rect { x: text.x + 1, width: text.width.saturating_sub(2), ..text },
    );
}

/// The window that moves HEAD. Each choice says what happens to the work
/// that has no commit.
fn render_reset(frame: &mut Frame, body: Rect, target: &str, subject: &str) {
    let area = centered(body, 70, 8);
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .title(Span::styled(
            " Move HEAD ",
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::new().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines = vec![
        Line::from(vec![
            Span::styled(format!("{target} "), Style::new().fg(Color::Yellow)),
            Span::styled(subject.to_string(), Style::new().fg(Color::White)),
        ]),
        Line::default(),
        Line::from(vec![
            Span::styled("s", KEY),
            Span::styled("  soft ", DESC),
            Span::styled(" your changes wait in the index", DIM),
        ]),
        Line::from(vec![
            Span::styled("m", KEY),
            Span::styled("  mixed", DESC),
            Span::styled("  your changes wait in the work tree", DIM),
        ]),
        Line::from(vec![
            Span::styled("h", KEY),
            Span::styled("  hard ", DESC),
            Span::styled(" your changes go away", Style::new().fg(Color::Red)),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner },
    );
}

/// The window that chooses what goes into a stash.
fn render_stash(frame: &mut Frame, body: Rect, path: Option<&str>) {
    let area = centered(body, 62, if path.is_some() { 8 } else { 7 });
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .title(Span::styled(" Stash ", Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)))
        .border_style(Style::new().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let row = |k: &'static str, what: &str, note: &str| {
        Line::from(vec![
            Span::styled(k, KEY),
            Span::styled(format!("  {what:<16}"), DESC),
            Span::styled(note.to_string(), DIM),
        ])
    };
    let mut lines = vec![
        row("a", "everything", "the changes git tracks"),
        row("u", "and new files", "those too"),
        row("s", "staged only", "what waits in the index"),
    ];
    if let Some(p) = path {
        lines.push(row("f", "this file", p));
    }
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner },
    );
}

/// The window that adds a path to the ignore rules. It offers the shared
/// file and the private one.
fn render_ignore(frame: &mut Frame, body: Rect, pattern: &str, tracked: bool) {
    let rows = if tracked { 8 } else { 6 };
    let area = centered(body, 66, rows);
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .title(Span::styled(" Ignore ", Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)))
        .border_style(Style::new().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = vec![
        Line::from(Span::styled(pattern.to_string(), Style::new().fg(Color::White))),
        Line::default(),
        Line::from(vec![
            Span::styled("i", KEY),
            Span::styled("  .gitignore", DESC),
            Span::styled("    every person who clones the repository", DIM),
        ]),
        Line::from(vec![
            Span::styled("e", KEY),
            Span::styled("  info/exclude", DESC),
            Span::styled("  only you", DIM),
        ]),
    ];
    // A rule does not remove a file that git already tracks.
    if tracked {
        lines.push(Line::default());
        lines.push(Line::styled(
            "git tracks this file, thus the rule does nothing until you",
            Style::new().fg(Color::Yellow),
        ));
        lines.push(Line::styled("stop tracking it.", Style::new().fg(Color::Yellow)));
    }
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner },
    );
}

/// The window that makes a worktree. It asks for a branch and a path. An
/// empty path takes the suggestion under the field.
fn render_new_worktree(
    frame: &mut Frame,
    body_area: Rect,
    branch: &str,
    path: &str,
    on_path: bool,
    app: &App,
) {
    let area = centered(body_area, 76, 12);
    frame.render_widget(Clear, area);
    let outer = Block::bordered()
        .title(Span::styled(
            " New worktree ",
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::new().fg(Color::Cyan));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let [top, mid, note, hint] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    let active = Style::new().fg(Color::Green);
    let idle = Style::new().fg(Color::DarkGray);
    let known = app.repo.branches.iter().any(|b| b.name == branch);
    let branch_title = if branch.is_empty() {
        "branch".to_string()
    } else if known {
        "branch (it exists)".to_string()
    } else {
        "branch (a new one)".to_string()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(branch.to_string(), Style::new().fg(Color::White)),
            Span::styled(if on_path { "" } else { "▏" }, active),
        ]))
        .block(Block::bordered().title(branch_title).border_style(if on_path { idle } else { active })),
        top,
    );
    // An empty path field shows the suggestion, in a dim color.
    let suggested = app.suggested_worktree_path(branch);
    let shown = if path.is_empty() && !on_path {
        Span::styled(suggested.clone(), idle)
    } else {
        Span::styled(path.to_string(), Style::new().fg(Color::White))
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            shown,
            Span::styled(if on_path { "▏" } else { "" }, active),
        ]))
        .block(Block::bordered().title("path").border_style(if on_path { active } else { idle })),
        mid,
    );
    frame.render_widget(
        Paragraph::new(if known {
            "The worktree will use the branch that is there."
        } else {
            "The worktree will make this branch."
        })
        .style(idle)
        .wrap(Wrap { trim: false }),
        Rect { x: note.x + 1, width: note.width.saturating_sub(2), ..note },
    );
    frame.render_widget(
        hint_line(&[&[("<enter>", "Make it"), ("<tab>", "Other field"), ("<esc>", "Cancel")]]),
        hint,
    );
}

/// The window that asks the user to say yes or no. A long path wraps, thus
/// the window grows to hold it.
fn render_confirm(frame: &mut Frame, body: Rect, prompt: &str) {
    let w = 60.min(body.width);
    let inner = w.saturating_sub(4).max(1) as usize;
    let text_rows = prompt.chars().count().div_ceil(inner).max(1) as u16;
    let area = centered(body, w, text_rows + 4);
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .title(Span::styled(
            " Confirm ",
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::new().fg(Color::Yellow));
    let text = block.inner(area);
    frame.render_widget(block, area);

    let [msg, hint] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(text);
    frame.render_widget(
        Paragraph::new(prompt.to_string())
            .wrap(Wrap { trim: false })
            .style(Style::new().fg(Color::White)),
        Rect { x: msg.x + 1, width: msg.width.saturating_sub(2), ..msg },
    );
    frame.render_widget(
        hint_line(&[&[("y", "Yes"), ("n / esc", "No")]]),
        Rect { x: hint.x + 1, width: hint.width.saturating_sub(2), ..hint },
    );
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
    &[("r", "Refresh")],
    &[
        ("<space>", "Stage"),
        ("a", "Stage all"),
        ("c", "Commit"),
        ("d", "Discard"),
        ("x", "Delete"),
        ("i", "Ignore"),
        ("s", "Stash"),
        ("<enter>", "Hunks"),
        ("e", "Open it"),
        ("o/t", "Ours/theirs"),
    ],
    &[
        ("<enter>", "Checkout"),
        ("n", "New"),
        ("F", "Force push"),
        ("d/D", "Delete"),
        ("R", "Rename"),
        ("m", "Merge"),
        ("P/p/f", "Push/pull/fetch"),
    ],
    &[
        ("<enter>", "Graph"),
        ("i", "Rebase"),
        ("w", "Reword"),
        ("v", "Revert"),
        ("y", "Cherry-pick"),
        ("f", "Fixup"),
        ("t", "Tag"),
        ("c", "Copy id"),
        ("R", "Reset"),
    ],
    &[("<enter>", "Apply"), ("p", "Pop"), ("d", "Drop")],
    &[("j/k", "Scroll"), ("g/G", "Top/bottom")],
];
const GLOBAL_HINTS: Hints = &[
    ("P/p/f", "Push/pull/fetch"),
    ("/", "Search"),
    (":", "Shell"),
    ("?", "Keys"),
    ("q", "Quit"),
];

// Make one line of "Desc: key | Desc: key", the shape lazygit uses.
fn hint_line(groups: &[Hints]) -> Line<'static> {
    hint_line_width(groups, usize::MAX)
}

/// The same line, but it stops before it goes past `width`. A hint that
/// does not fit stays out, thus the bar never runs off the screen.
fn hint_line_width(groups: &[Hints], width: usize) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    let mut dropped = false;
    for group in groups {
        for (key, desc) in group.iter() {
            let sep = if spans.is_empty() { 0 } else { 3 };
            let need = sep + desc.len() + 2 + key.len();
            // Keep one column free for the mark that says "there is more".
            if used + need > width.saturating_sub(2) {
                dropped = true;
                break;
            }
            if sep > 0 {
                spans.push(Span::styled(" | ", DIM));
            }
            spans.push(Span::styled(format!("{desc}: "), DESC));
            spans.push(Span::styled((*key).to_string(), KEY));
            used += need;
        }
    }
    if dropped {
        spans.push(Span::styled(" …", DIM));
    }
    Line::from(spans)
}

fn render_bar(frame: &mut Frame, area: Rect, app: &App) {
    let w = area.width as usize;
    let line = match &app.mode {
        Mode::Input { prompt, buffer, .. } => Line::from(vec![
            Span::styled(format!("{prompt}: "), Style::new().fg(Color::Cyan)),
            Span::styled(format!("{buffer}▏"), Style::new().fg(Color::White)),
        ]),
        // The confirm window draws its own text and keys.
        Mode::Confirm { .. } => Line::default(),
        Mode::Hunks { picked, .. } => {
            let mut spans = vec![Span::styled(
                format!("{} marked  ", picked.len()),
                Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            )];
            spans.extend(
                hint_line(&[&[
                    ("<space>", "Mark line"),
                    ("<enter>", "Stage marked"),
                    ("a", "Stage hunk"),
                    ("J/K", "Hunk"),
                    ("<esc>", "Back"),
                ]])
                .spans,
            );
            Line::from(spans)
        }
        Mode::Rebase { .. } => hint_line(&[&[
            ("p", "Pick"),
            ("r", "Reword"),
            ("e", "Edit"),
            ("s", "Squash"),
            ("f", "Fixup"),
            ("d", "Drop"),
            ("J/K", "Move"),
            ("<enter>", "Run"),
            ("<esc>", "Cancel"),
        ]]),
        Mode::Help => Line::styled("press any key to close the help", Style::new().fg(Color::Cyan)),
        Mode::Worktrees { .. } => hint_line(&[&[
            ("<enter>", "Go to it"),
            ("n", "New"),
            ("d", "Remove"),
            ("p", "Prune"),
            ("<esc>", "Close"),
        ]]),
        // The window draws its own keys.
        Mode::NewWorktree { .. } => Line::default(),
        // The window draws its own note.
        Mode::Error { .. } => Line::default(),
        Mode::Reset { .. } => hint_line(&[&[
            ("s", "Keep in the index"),
            ("m", "Keep in the work tree"),
            ("h", "Throw away"),
            ("<esc>", "Cancel"),
        ]]),
        Mode::Stash { .. } => hint_line(&[&[
            ("a", "All"),
            ("u", "With new files"),
            ("s", "Staged only"),
            ("f", "This file"),
            ("<esc>", "Cancel"),
        ]]),
        Mode::Blame { .. } => hint_line(&[&[
            ("j/k", "Move"),
            ("d/u", "Page"),
            ("g/G", "Top/end"),
            ("<esc>", "Close"),
        ]]),
        Mode::Submodules { .. } => hint_line(&[&[
            ("<enter>", "Update this one"),
            ("u", "Update all"),
            ("<esc>", "Close"),
        ]]),
        Mode::Reflog { .. } => hint_line(&[&[
            ("<enter>", "Move HEAD there"),
            ("j/k", "Move"),
            ("<esc>", "Close"),
        ]]),
        Mode::Ignore { .. } => hint_line(&[&[
            ("i", "Share the rule"),
            ("e", "Keep it to yourself"),
            ("<esc>", "Cancel"),
        ]]),
        // A bisect takes over four keys of the commits panel.
        Mode::Normal if app.repo.bisecting => {
            let mut spans = vec![Span::styled(
                "BISECT  ",
                Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            )];
            spans.extend(
                hint_line(&[&[("b", "Bad"), ("o", "Good"), ("S", "Skip"), ("A", "Reset")]]).spans,
            );
            Line::from(spans)
        }
        // The commit window draws its own key hints.
        Mode::CommitMsg { .. } => Line::default(),
        // A stopped rebase takes over three keys, but not on the files
        // panel, where you stage and commit the parts of the work.
        Mode::Normal if app.rebase.is_some() => {
            let r = app.rebase.as_ref().unwrap();
            let word = if app.splitting { "SPLIT" } else { "REBASE" };
            let mut spans = vec![Span::styled(
                format!("{word} {}/{}  ", r.step, r.total),
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )];
            let keys: Hints = if app.focus == 1 {
                &[("<space>", "Stage a part"), ("c", "Commit it"), ("[1] then c", "Go on")]
            } else {
                &[("c", "Continue"), ("s", "Skip"), ("A", "Abort")]
            };
            spans.extend(hint_line(&[keys]).spans);
            Line::from(spans)
        }
        // A failed command shows its message in red.
        Mode::Normal if !app.message.is_empty() => {
            let color = if app.message_ok { Color::Green } else { Color::Red };
            Line::styled(app.message.clone(), Style::new().fg(color))
        }
        // A running network command comes first, thus you always see it.
        Mode::Normal if !app.running.is_empty() => {
            let frame = app.spinner().unwrap_or(' ');
            let mut spans = vec![Span::styled(
                format!("{frame} {}  ", app.running.join(", ")),
                Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )];
            let left = w.saturating_sub(spans[0].content.chars().count());
            spans.extend(hint_line_width(&[GLOBAL_HINTS], left).spans);
            Line::from(spans)
        }
        Mode::Normal => hint_line_width(&[HINTS[app.focus.min(5)], GLOBAL_HINTS], w),
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
            ("/ esc", "search the commit messages, or show them all"),
            (":", "run a command line through the shell"),
            ("P / p / f", "push, pull, or fetch, in the background"),
            ("F", "force push, with a lease that protects other people"),
            ("W / M / U", "worktrees, submodules, where HEAD has been"),
            ("? / @ / q", "help, command log, quit"),
        ],
    ),
    (
        "Files [2]",
        &[
            ("space", "stage or unstage the file or the directory"),
            ("a", "stage all files"),
            ("enter", "open the hunks, or fold the directory"),
            ("d", "discard the changes (it asks first)"),
            ("x", "delete it from the disk (it asks first)"),
            ("i", "ignore it: i shares the rule, e keeps it to yourself"),
            ("A", "add the staged changes to the last commit"),
            ("z", "send each staged change back to the commit that wrote it"),
            ("e", "open the file in your editor"),
            ("b", "see who last changed each line"),
            ("c / C", "open the commit window, or use the editor"),
            ("s", "stash: it asks which changes to put away"),
            ("o / t", "take ours or theirs in a conflict"),
            ("v", "mark a file, thus the next key works on every mark"),
            ("esc", "drop the marks"),
            ("SM name", "the two marks of git status, then the file"),
        ],
    ),
    (
        "Branches [3]",
        &[
            ("enter", "check out the branch, if you are not on it"),
            ("n", "make a new branch"),
            ("d / D", "delete it, or delete it by force"),
            ("R", "give the branch a new name"),
            ("m", "merge the branch into the current one"),
            ("o", "put your commits on top of that branch"),
            ("* ↑ ↓", "current branch, to push, to pull"),
        ],
    ),
    (
        "Submodules",
        &[("M", "open the list"), ("enter / u", "update this one, or every one")],
    ),
    (
        "Go back",
        &[
            ("U", "the list of places HEAD has been"),
            ("R", "on a commit: move HEAD to it"),
            ("s / m / h", "keep in the index, in the work tree, or not"),
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
            ("s", "open the commit, so it can become several commits"),
            ("w", "give the commit a new message"),
            ("v", "revert the commit"),
            ("y", "put its changes in the index, with no commit"),
            ("f", "fold the staged changes into this commit"),
            ("t / T", "make a tag here, or push every tag"),
            ("c", "copy the id to the clipboard"),
            ("/ esc", "search the messages, or show them all again"),
            ("space", "mark a commit, then move to see what lies between"),
            ("↑", "this commit is not on the upstream branch"),
            ("⬟", "a commit that carries a tag"),
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
            ("n", "make one: it asks for a branch and a path"),
            ("d / p", "remove one, or drop the records of lost ones"),
        ],
    ),
    (
        "Stash [5]",
        &[
            ("enter / a", "apply the stash"),
            ("p / d", "pop it, or drop it"),
            ("a u s f", "in the stash window: all, new files too, staged, one file"),
        ],
    ),
    (
        "Diff pane [0]",
        &[
            ("j / k", "scroll it"),
            ("g / G", "go to the top or the end"),
            ("marked words", "only the words that changed are marked"),
        ],
    ),
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
