use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};

use super::list;
use crate::app::{App, PANELS};
use crate::git::{BranchEntry, CommitEntry};
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

/// The glyphs of one graph row. A tagged commit gets a diamond, thus a
/// release stands out from the commits around it.
fn graph_spans(graph: &str, tagged: bool) -> Vec<Span<'static>> {
    graph
        .chars()
        .enumerate()
        .map(|(i, ch)| {
            let lane = Style::new().fg(LANE_COLORS[(i / 2) % LANE_COLORS.len()]);
            match ch {
                '●' | '◉' if tagged => Span::styled(
                    "◆".to_string(),
                    Style::new().fg(Color::LightYellow).add_modifier(Modifier::BOLD),
                ),
                _ => Span::styled(ch.to_string(), lane),
            }
        })
        .collect()
}

// Two letters from the name of the author, as lazygit shows them.
fn initials(author: &str) -> String {
    let mut out = String::new();
    for word in author.split_whitespace().take(2) {
        if let Some(c) = word.chars().next() {
            out.extend(c.to_uppercase());
        }
    }
    if out.is_empty() { "??".into() } else { format!("{out:<2}") }
}

// Give each author a steady color. The sum of the letters picks it.
fn author_color(author: &str) -> Color {
    const PALETTE: [Color; 6] = [
        Color::LightMagenta,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightCyan,
        Color::LightBlue,
        Color::LightRed,
    ];
    let sum: u32 = author.bytes().map(u32::from).sum();
    PALETTE[sum as usize % PALETTE.len()]
}

fn commit_line(c: &CommitEntry, zoomed: bool, unpushed: bool, tags: &[String]) -> Line<'static> {
    // The order matches lazygit: id, author, graph, then the subject.
    let mut spans = vec![
        Span::styled(c.id_str().to_string(), Style::new().fg(Color::Yellow)),
        Span::raw(" "),
        Span::styled(initials(&c.author), Style::new().fg(author_color(&c.author))),
        Span::raw(" "),
    ];
    if zoomed {
        spans.push(Span::styled(format!("{} ", ymd(c.time)), Style::new().fg(Color::DarkGray)));
    }
    spans.extend(graph_spans(&c.graph, !tags.is_empty()));
    // An up arrow marks a commit that the upstream branch does not have.
    if unpushed {
        spans.push(Span::styled("↑", Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
    }
    // A tag sits before the subject, as git shows it.
    for t in tags {
        spans.push(Span::styled(
            format!(" {t}"),
            Style::new().fg(Color::LightYellow).add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::raw(format!(" {}", c.subject)));
    Line::from(spans)
}

// The tags of a commit, or nothing.
fn tags_of<'a>(app: &'a App, c: &CommitEntry) -> &'a [String] {
    app.repo.tags.get(c.id_str()).map(Vec::as_slice).unwrap_or(&[])
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

// A branch row: a mark for the current branch, then the name, then the
// number of commits to push and to pull.
// Cut a name that is too long. The last character becomes an ellipsis.
fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(1);
    text.chars().take(keep).chain(std::iter::once('…')).collect()
}

/// One branch row. The counts to push and to pull always show. The name
/// gives up its own columns to make room for them.
fn branch_line(b: &BranchEntry, width: usize, spinner: Option<char>) -> Line<'static> {
    // A network command works on the branch you are on, thus its mark
    // becomes the spinner while the command runs.
    let (icon, icon_color, name_style) = match (b.current, spinner) {
        (true, Some(frame)) => (
            frame.to_string(),
            Color::Cyan,
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        (true, None) => (
            "*".to_string(),
            Color::Green,
            Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
        ),
        _ => (" ".to_string(), Color::Green, Style::new()),
    };
    // Build the tail first, thus its width is known.
    let mut tail: Vec<Span<'static>> = Vec::new();
    if b.ahead > 0 {
        tail.push(Span::styled(format!(" ↑{}", b.ahead), Style::new().fg(Color::Yellow)));
    }
    if b.behind > 0 {
        tail.push(Span::styled(format!(" ↓{}", b.behind), Style::new().fg(Color::Magenta)));
    }
    if b.gone {
        tail.push(Span::styled(" gone", Style::new().fg(Color::Red)));
    }
    let tail_width: usize = tail.iter().map(|s| s.content.chars().count()).sum();
    // The age takes five columns and the mark takes two.
    let room = width.saturating_sub(7 + tail_width).max(3);

    // A branch that lives on a remote shows in a quieter color.
    let name_style = if b.remote { Style::new().fg(Color::DarkGray) } else { name_style };
    let mut spans = vec![
        // The age column comes first, as lazygit shows it.
        Span::styled(format!("{:>4} ", b.age), Style::new().fg(Color::Cyan)),
        Span::styled(icon, Style::new().fg(icon_color)),
        Span::styled(format!(" {}", shorten(&b.name, room)), name_style),
    ];
    spans.extend(tail);
    Line::from(spans)
}

// A tree row: a folded or open directory, or a file with its mark.
fn tree_line(app: &App, i: usize) -> Line<'static> {
    let row = &app.tree[i];
    let pad = "  ".repeat(row.depth as usize);
    if let Some(dir) = &row.dir {
        let arrow = if app.collapsed.contains(dir) { '▸' } else { '▾' };
        // The root row already carries its slash.
        let slash = if dir.is_empty() { "" } else { "/" };
        return Line::styled(
            format!("   {pad}{arrow} {}{slash}", row.name),
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        );
    }
    let f = &app.repo.files[row.file.unwrap_or(0)];
    // The name says the state of the file on its own:
    //   bold green  the index holds every change
    //   yellow      part is in the index, part is not
    //   red         nothing is in the index
    //   dim red     git does not track the file
    let name_style = if f.conflicted() {
        Style::new().fg(Color::LightRed).add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else if f.staged() && f.work == ' ' {
        Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else if f.staged() {
        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else if f.work == '?' {
        Style::new().fg(Color::Red).add_modifier(Modifier::DIM)
    } else {
        Style::new().fg(Color::Red)
    };
    Line::from(vec![
        // The first column is the index. The second is the work tree.
        Span::styled(
            f.index.to_string(),
            Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
        ),
        Span::styled(f.work.to_string(), Style::new().fg(mark_color(f.work))),
        Span::styled(format!(" {pad}{}", row.name), name_style),
    ])
}

pub fn render_left(frame: &mut Frame, areas: [Rect; 5], app: &App) {
    let repo = &app.repo;
    // The border takes two columns of the branch panel.
    let branch_width = areas[2].width.saturating_sub(2) as usize;
    let spinner = app.spinner();
    let mut head = repo.head.clone().unwrap_or_else(|| "(no repo)".into());
    // Show how far the branch is from its upstream.
    if repo.ahead > 0 {
        head.push_str(&format!(" ↑{}", repo.ahead));
    }
    if repo.behind > 0 {
        head.push_str(&format!(" ↓{}", repo.behind));
    }
    // A rebase or a bisect replaces the branch name with its state.
    let banner = app
        .rebase
        .as_ref()
        .map(|r| (format!("REBASE {}/{}", r.step, r.total), Color::Yellow))
        .or_else(|| repo.bisecting.then(|| ("BISECT".to_string(), Color::Magenta)));
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
        &|i| branch_line(&repo.branches[i], branch_width, spinner),
        &|i| {
            let c = &repo.commits[i];
            commit_line(c, false, repo.unpushed.contains(c.id_str()), tags_of(app, c))
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
    // Each title carries the key that focuses the panel. A search shows in
    // the title of the commits panel, thus the list is never a surprise.
    for (i, area) in areas.into_iter().enumerate() {
        let title = match (i, &repo.filter) {
            (3, Some(text)) => format!("[4]─Commits─search: {text}"),
            _ => format!("[{}]─{}", i + 1, PANELS[i]),
        };
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
            commit_line(c, true, repo.unpushed.contains(c.id_str()), tags_of(app, c))
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

// Color one line of a diff.
fn diff_line(l: &str) -> Line<'static> {
    // File header lines come before the +/- check. A "---" line is a
    // header, not a removal.
    let style = if l.starts_with("diff ")
        || l.starts_with("index ")
        || l.starts_with("--- ")
        || l.starts_with("+++ ")
    {
        Style::new().add_modifier(Modifier::BOLD)
    } else if l.starts_with("commit ")
        || l.starts_with("Author")
        || l.starts_with("Date")
        || l.starts_with("Merge")
    {
        Style::new().fg(Color::Yellow)
    } else {
        match l.as_bytes().first() {
            Some(b'+') => Style::new().fg(Color::Green),
            Some(b'-') => Style::new().fg(Color::Red),
            Some(b'@') => Style::new().fg(Color::Cyan),
            _ => Style::new(),
        }
    };
    Line::styled(l.to_string(), style)
}

/// One hunk, with a cursor on a line and a mark on each picked line.
fn render_hunk(
    frame: &mut Frame,
    area: Rect,
    path: &str,
    hunks: &[String],
    cursor: usize,
    line: usize,
    picked: &[usize],
) {
    let body: Vec<&str> = hunks[cursor].lines().skip(1).collect();
    let title = format!("Hunk {}/{} — {path}", cursor + 1, hunks.len());
    list::render(frame, area, &title, true, line, body.len(), &|i| {
        let text = body[i];
        let style = match text.as_bytes().first() {
            Some(b'+') => Style::new().fg(Color::Green),
            Some(b'-') => Style::new().fg(Color::Red),
            _ => Style::new().fg(Color::Gray),
        };
        // A marked line shows a bar in the gutter.
        let gutter = if picked.contains(&i) {
            Span::styled("▌", Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        } else {
            Span::raw(" ")
        };
        Line::from(vec![gutter, Span::styled(text.to_string(), style)])
    });
}

pub fn render_main(frame: &mut Frame, area: Rect, app: &App) {
    // The rebase editor and the hunk view replace the diff pane.
    match &app.mode {
        crate::app::Mode::Rebase { items, cursor, .. } => {
            return render_rebase(frame, area, items, *cursor);
        }
        crate::app::Mode::Hunks { path, hunks, cursor, line, picked, .. } => {
            return render_hunk(frame, area, path, hunks, *cursor, *line, picked);
        }
        _ => {}
    }
    let both = !app.repo.diff_staged.is_empty() && !app.repo.diff.is_empty();
    let title = match (app.focus, both) {
        (3, _) => "[0]─Commit",
        (_, true) => "[0]─Changes",
        _ if !app.repo.diff_staged.is_empty() => "[0]─Staged changes",
        _ => "[0]─Unstaged changes",
    };

    // One pane holds both sides. A header row separates them, thus the
    // whole height is free for the diff.
    let mut rows: Vec<Line> = Vec::new();
    let width = area.width.saturating_sub(2) as usize;
    let limit = app.diff_scroll as usize + area.height as usize * 4;
    for (name, text, color) in [
        ("Unstaged changes", &app.repo.diff, Color::Red),
        ("Staged changes", &app.repo.diff_staged, Color::Green),
    ] {
        if text.is_empty() {
            continue;
        }
        if both {
            // "── " and the space after the name use four columns.
            let bar = "─".repeat(width.saturating_sub(name.len() + 4));
            rows.push(Line::styled(
                format!("── {name} {bar}"),
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            ));
        }
        for l in text.lines() {
            rows.push(diff_line(l));
            if rows.len() >= limit {
                break;
            }
        }
        if rows.len() >= limit {
            break;
        }
    }
    // A wrapped line can use more than one row. Take more lines than the
    // height, then let the widget cut what does not fit.
    let lines: Vec<Line> = rows.into_iter().skip(app.diff_scroll as usize).collect();
    let border = if app.focus == 5 { Color::Green } else { Color::DarkGray };
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .title(Span::styled(
                    title.to_string(),
                    Style::new().fg(border).add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::new().fg(border)),
        ),
        area,
    );
}

/// The worktree list. It opens over the panels.
pub fn render_worktrees(
    frame: &mut Frame,
    area: Rect,
    list: &[crate::git::WorktreeEntry],
    cursor: usize,
    home: &str,
) {
    frame.render_widget(Clear, area);
    let width = area.width.saturating_sub(2) as usize;
    list::render(frame, area, " Worktrees ", true, cursor, list.len(), &|i| {
        let w = &list[i];
        // A tag says what is special about the worktree.
        let mut tag = String::new();
        if w.main {
            tag.push_str(" main");
        }
        if w.locked {
            tag.push_str(" locked");
        }
        if w.prunable {
            tag.push_str(" gone");
        }
        // A path under your home directory shows as a tilde.
        let path = match (!home.is_empty()).then(|| w.path.strip_prefix(home)).flatten() {
            Some(rest) => format!("~{rest}"),
            None => w.path.clone(),
        };
        let branch_col = 22.min(width / 3);
        let room = width.saturating_sub(2 + branch_col + tag.chars().count() + 1).max(6);
        let (icon, branch_style) = if w.current {
            ("●", Style::new().fg(Color::Green).add_modifier(Modifier::BOLD))
        } else {
            (" ", Style::new().fg(Color::Cyan))
        };
        Line::from(vec![
            Span::styled(icon, Style::new().fg(Color::Green)),
            Span::styled(
                format!(" {:<w$.w$}", w.branch, w = branch_col),
                branch_style,
            ),
            Span::styled(shorten(&path, room), Style::new().fg(Color::Gray)),
            Span::styled(tag, Style::new().fg(if w.prunable { Color::Red } else { Color::DarkGray })),
        ])
    });
}

/// The recent positions of HEAD. It is the way back from a mistake.
pub fn render_reflog(
    frame: &mut Frame,
    area: Rect,
    list: &[crate::git::ReflogEntry],
    cursor: usize,
) {
    frame.render_widget(Clear, area);
    let width = area.width.saturating_sub(2) as usize;
    list::render(frame, area, " Where HEAD has been ", true, cursor, list.len(), &|i| {
        let e = &list[i];
        let room = width.saturating_sub(e.id.len() + e.at.len() + 3).max(6);
        Line::from(vec![
            Span::styled(format!("{} ", e.id), Style::new().fg(Color::Yellow)),
            Span::styled(format!("{:<14.14} ", e.at), Style::new().fg(Color::Cyan)),
            Span::styled(shorten(&e.what, room), Style::new().fg(Color::Gray)),
        ])
    });
}

/// The command log shows the last git commands, their result, and how long
/// each one took.
pub fn render_log(frame: &mut Frame, area: Rect, app: &App) {
    let take = area.height.saturating_sub(2) as usize;
    let width = area.width.saturating_sub(2).max(1) as usize;
    // A wrapped line uses more than one row. Count the rows it will use.
    let height = |line: &Line| line.width().div_ceil(width).max(1);
    // A failed command needs two rows: the command and the reason.
    let rows = |e: &crate::app::LogEntry| {
        let (icon, color) = if e.ok { ("✓", Color::Green) } else { ("✗", Color::Red) };
        // A slow command gets a warm color, thus it is easy to see.
        let time_color = if e.ms >= 500 { Color::Yellow } else { Color::DarkGray };
        let mut out = vec![Line::from(vec![
            Span::styled(format!("{icon} "), Style::new().fg(color)),
            Span::styled(e.cmd.clone(), Style::new().fg(if e.ok { Color::Gray } else { Color::Red })),
            Span::styled(format!("  {}ms", e.ms), Style::new().fg(time_color)),
        ])];
        let color = if e.ok { Color::DarkGray } else { Color::Red };
        for line in &e.output {
            out.push(Line::styled(format!("  {line}"), Style::new().fg(color)));
        }
        out
    };
    // Count from the newest entry back. The newest entry always shows,
    // even when its output alone is taller than the pane.
    let mut lines: Vec<Line> = Vec::new();
    let mut used = 0usize;
    for e in app.cmd_log.iter().rev() {
        let mut r = rows(e);
        let need: usize = r.iter().map(&height).sum();
        if used + need > take {
            if lines.is_empty() {
                // Keep the rows of the newest entry that fit.
                let mut fit = 0;
                while fit < r.len() && used + height(&r[fit]) <= take {
                    used += height(&r[fit]);
                    fit += 1;
                }
                r.truncate(fit.max(1));
                lines = r;
            }
            break;
        }
        used += need;
        r.extend(lines);
        lines = r;
    }
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .title(Span::styled("[@]─Command log", Style::new().fg(Color::DarkGray)))
                .border_style(Style::new().fg(Color::DarkGray)),
        ),
        area,
    );
}
