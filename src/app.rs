use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{KeyCode, KeyEventKind};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use std::collections::HashSet;

use crate::event::{self, Msg};
use crate::git::{self, CommitEntry, DiffTarget, FileEntry, Git, Req, Resp, patch};
use crate::keys::{Action, action_for};
use crate::tree::{self, TreeRow};
use crate::ui;

pub const PANELS: [&str; 5] = ["Status", "Files", "Branches", "Commits", "Stash"];
// Load commits in small chunks. A large chunk touches many pack pages at
// start time. The scroll logic requests the next chunk early enough.
const LOG_CHUNK: usize = 100;

#[derive(Default)]
pub struct RepoState {
    pub head: Option<String>,
    pub files: Vec<FileEntry>,
    pub branches: Vec<String>,
    pub commits: Vec<CommitEntry>,
    pub stashes: Vec<String>,
    pub log_done: bool,
    pub diff: String,
    pub ahead: u32,
    pub behind: u32,
    /// Short ids of commits that the upstream branch does not have.
    pub unpushed: HashSet<String>,
}

pub enum InputPurpose {
    CommitMsg,
    NewBranch,
    StashMsg,
}

pub enum ConfirmAction {
    DeleteBranch(String),
    DropStash(usize),
}

pub enum Mode {
    Normal,
    Input { prompt: &'static str, buffer: String, purpose: InputPurpose },
    Confirm { prompt: String, action: ConfirmAction },
    Hunks { path: String, header: String, hunks: Vec<String>, cursor: usize },
    Help,
}

pub struct App {
    /// Focus 0 to 4 is a left panel. Focus 5 is the diff pane.
    pub focus: usize,
    pub selected: [usize; 6],
    pub quit: bool,
    pub repo: RepoState,
    pub mode: Mode,
    pub message: String,
    pub message_ok: bool,
    pub zoom: bool,
    pub diff_scroll: u16,
    pub tree: Vec<TreeRow>,
    pub collapsed: HashSet<String>,
    pub cmd_log: Vec<(bool, String)>,
    pub show_log: bool,
    git: Option<Git>,
    log_inflight: bool,
    diff_seq: u64,
    diff_target: Option<DiffTarget>,
    pending_suspend: Option<Vec<String>>,
    pause: Arc<AtomicBool>,
}

impl App {
    pub fn new() -> Self {
        Self {
            focus: 1,
            selected: [0; 6],
            quit: false,
            repo: RepoState::default(),
            mode: Mode::Normal,
            message: String::new(),
            message_ok: true,
            zoom: false,
            diff_scroll: 0,
            tree: Vec::new(),
            collapsed: HashSet::new(),
            cmd_log: Vec::new(),
            show_log: true,
            git: None,
            log_inflight: false,
            diff_seq: 0,
            diff_target: None,
            pending_suspend: None,
            pause: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        event::spawn_input(tx.clone(), self.pause.clone());
        let git = git::spawn(tx.clone())?;
        git::watch::spawn(git.git_dir.clone(), tx);
        self.git = Some(git);
        self.refresh_all();

        while !self.quit {
            terminal.draw(|f| ui::render(f, self))?;
            // Drain the queue before each draw. This makes one draw for a
            // burst of messages, not one draw for each message.
            self.update(rx.recv()?);
            while let Ok(msg) = rx.try_recv() {
                self.update(msg);
            }
            self.flush_requests();
            if let Some(args) = self.pending_suspend.take() {
                self.suspend_and_run(terminal, args)?;
            }
        }
        Ok(())
    }

    /// Give the terminal to a git child process, for example push or an
    /// editor for a commit message. Restore the terminal after it.
    fn suspend_and_run(&mut self, terminal: &mut DefaultTerminal, args: Vec<String>) -> Result<()> {
        let Some(git) = &self.git else { return Ok(()) };
        self.pause.store(true, Ordering::Relaxed);
        disable_raw_mode()?;
        execute!(std::io::stdout(), LeaveAlternateScreen)?;
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&git.root)
            .args(&args)
            .status();
        enable_raw_mode()?;
        execute!(std::io::stdout(), EnterAlternateScreen)?;
        terminal.clear()?;
        self.pause.store(false, Ordering::Relaxed);
        self.message_ok = matches!(&status, Ok(s) if s.success());
        self.message = match status {
            Ok(s) if s.success() => format!("git {} done", args.join(" ")),
            Ok(s) => format!("git {} failed ({s})", args.join(" ")),
            Err(e) => e.to_string(),
        };
        self.refresh_all();
        Ok(())
    }

    pub fn update(&mut self, msg: Msg) {
        match msg {
            Msg::Key(key) if key.kind == KeyEventKind::Press => self.handle_key(key),
            Msg::Git(resp) => self.apply_resp(resp),
            Msg::Refresh => self.refresh_all(),
            // A resize needs no work. The next draw uses the new size.
            _ => {}
        }
    }

    fn handle_key(&mut self, key: ratatui::crossterm::event::KeyEvent) {
        match &mut self.mode {
            Mode::Normal => {
                // A key press removes the old message. The bar then shows
                // the key hints again.
                self.message.clear();
                if let Some(action) = action_for(key, self.focus) {
                    self.apply(action);
                }
            }
            Mode::Help => self.mode = Mode::Normal,
            Mode::Input { buffer, .. } => match key.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Backspace => {
                    buffer.pop();
                }
                KeyCode::Char(c) => buffer.push(c),
                KeyCode::Enter => {
                    let Mode::Input { buffer, purpose, .. } = std::mem::replace(&mut self.mode, Mode::Normal) else {
                        return;
                    };
                    self.submit_input(purpose, buffer);
                }
                _ => {}
            },
            Mode::Confirm { .. } => match key.code {
                KeyCode::Char('y') => {
                    let Mode::Confirm { action, .. } = std::mem::replace(&mut self.mode, Mode::Normal) else {
                        return;
                    };
                    let args: Vec<String> = match action {
                        ConfirmAction::DeleteBranch(name) => svec(&["branch", "-D", &name]),
                        ConfirmAction::DropStash(i) => svec(&["stash", "drop", &format!("stash@{{{i}}}")]),
                    };
                    self.write(args);
                }
                _ => self.mode = Mode::Normal,
            },
            Mode::Hunks { header, hunks, cursor, .. } => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.mode = Mode::Normal,
                KeyCode::Char('j') | KeyCode::Down => *cursor = (*cursor + 1).min(hunks.len() - 1),
                KeyCode::Char('k') | KeyCode::Up => *cursor = cursor.saturating_sub(1),
                KeyCode::Char(' ') => {
                    let patch = patch::hunk_patch(header, &hunks[*cursor]);
                    // Line numbers of the other hunks change after the
                    // apply. Leave the mode instead of a stale re-use.
                    self.mode = Mode::Normal;
                    if let Some(git) = &self.git {
                        git.send(Req::ApplyPatch { patch, reverse: false });
                    }
                }
                _ => {}
            },
        }
    }

    fn submit_input(&mut self, purpose: InputPurpose, buffer: String) {
        if buffer.is_empty() {
            return;
        }
        let args: Vec<String> = match purpose {
            InputPurpose::CommitMsg => svec(&["commit", "-m", &buffer]),
            InputPurpose::NewBranch => svec(&["checkout", "-b", &buffer]),
            InputPurpose::StashMsg => svec(&["stash", "push", "-m", &buffer]),
        };
        self.write(args);
    }

    fn write(&mut self, args: Vec<String>) {
        self.log_cmd(true, format!("→ git {}", args.join(" ")));
        if let Some(git) = &self.git {
            git.send(Req::Write(args));
        }
    }

    fn log_cmd(&mut self, ok: bool, line: String) {
        self.cmd_log.push((ok, line));
        // Keep the log short. Old entries have no value.
        if self.cmd_log.len() > 100 {
            self.cmd_log.remove(0);
        }
    }

    pub fn panel_len(&self, panel: usize) -> usize {
        match panel {
            0 => 1,
            1 => self.tree.len(),
            2 => self.repo.branches.len(),
            3 => self.repo.commits.len(),
            4 => self.repo.stashes.len(),
            _ => 0,
        }
    }

    fn selected_row(&self) -> Option<&TreeRow> {
        self.tree.get(self.selected[1])
    }

    fn selected_file(&self) -> Option<&FileEntry> {
        self.selected_row().and_then(|r| r.file).and_then(|i| self.repo.files.get(i))
    }

    fn rebuild_tree(&mut self) {
        self.tree = tree::build(&self.repo.files, &self.collapsed);
        self.clamp(1);
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Quit => self.quit = true,
            Action::NextPanel => self.focus = (self.focus + 1) % 6,
            Action::PrevPanel => self.focus = (self.focus + 5) % 6,
            Action::FocusPanel(i) => self.focus = i,
            // In the diff pane, the motion keys scroll the text.
            Action::Down if self.focus == 5 => self.diff_scroll = self.diff_scroll.saturating_add(1),
            Action::Up if self.focus == 5 => self.diff_scroll = self.diff_scroll.saturating_sub(1),
            Action::PageDown if self.focus == 5 => self.diff_scroll = self.diff_scroll.saturating_add(15),
            Action::PageUp if self.focus == 5 => self.diff_scroll = self.diff_scroll.saturating_sub(15),
            Action::Top if self.focus == 5 => self.diff_scroll = 0,
            Action::Down => {
                let len = self.panel_len(self.focus);
                let sel = &mut self.selected[self.focus];
                if *sel + 1 < len {
                    *sel += 1;
                }
            }
            Action::Up => {
                let sel = &mut self.selected[self.focus];
                *sel = sel.saturating_sub(1);
            }
            // ponytail: the page step is a constant. The exact panel height
            // is not worth the plumbing.
            Action::PageDown => {
                let len = self.panel_len(self.focus);
                let sel = &mut self.selected[self.focus];
                *sel = (*sel + 15).min(len.saturating_sub(1));
            }
            Action::PageUp => {
                let sel = &mut self.selected[self.focus];
                *sel = sel.saturating_sub(15);
            }
            Action::Top => self.selected[self.focus] = 0,
            Action::Bottom => {
                self.selected[self.focus] = self.panel_len(self.focus).saturating_sub(1);
            }
            Action::DiffScroll(delta) => {
                self.diff_scroll = self.diff_scroll.saturating_add_signed(delta as i16);
            }
            Action::ZoomGraph => self.zoom = !self.zoom,
            Action::Help => self.mode = Mode::Help,
            Action::ToggleLog => self.show_log = !self.show_log,
            Action::Refresh => self.refresh_all(),

            Action::ToggleStage => {
                // On a directory row, stage the whole directory.
                if let Some(dir) = self.selected_row().and_then(|r| r.dir.clone()) {
                    self.write(svec(&["add", "--", &dir]));
                } else if let Some(f) = self.selected_file() {
                    let args = if f.staged {
                        svec(&["restore", "--staged", "--", &f.path])
                    } else {
                        svec(&["add", "--", &f.path])
                    };
                    self.write(args);
                }
            }
            Action::StageAll => self.write(svec(&["add", "-A"])),
            Action::CommitPrompt => {
                self.mode = Mode::Input { prompt: "commit message", buffer: String::new(), purpose: InputPurpose::CommitMsg };
            }
            Action::CommitEditor => self.pending_suspend = Some(svec(&["commit"])),
            Action::StashPrompt => {
                self.mode = Mode::Input { prompt: "stash message", buffer: String::new(), purpose: InputPurpose::StashMsg };
            }
            Action::EnterHunks => {
                // On a directory row, the enter key folds or unfolds it.
                if let Some(dir) = self.selected_row().and_then(|r| r.dir.clone()) {
                    if !self.collapsed.remove(&dir) {
                        self.collapsed.insert(dir);
                    }
                    self.rebuild_tree();
                    return;
                }
                let Some(f) = self.selected_file() else { return };
                // The diff pane must already show this file. The diff text
                // is index-to-worktree, thus the hunks fit `apply --cached`.
                if self.diff_target != Some(DiffTarget::WorktreeFile(f.path.clone())) {
                    return;
                }
                match patch::split_diff(&self.repo.diff) {
                    Some((header, hunks)) if !hunks.is_empty() => {
                        self.mode = Mode::Hunks { path: f.path.clone(), header, hunks, cursor: 0 };
                    }
                    _ => self.message = "no hunks in this file".into(),
                }
            }
            Action::TakeOurs | Action::TakeTheirs => {
                let side = if matches!(action, Action::TakeOurs) { "--ours" } else { "--theirs" };
                if let Some(f) = self.selected_file()
                    && f.mark == 'U'
                {
                    let path = f.path.clone();
                    self.write(svec(&["checkout", side, "--", &path]));
                    self.write(svec(&["add", "--", &path]));
                }
            }

            Action::Checkout => {
                if let Some(name) = self.repo.branches.get(self.selected[2]) {
                    let name = name.clone();
                    self.write(svec(&["checkout", &name]));
                }
            }
            Action::NewBranchPrompt => {
                self.mode = Mode::Input { prompt: "new branch name", buffer: String::new(), purpose: InputPurpose::NewBranch };
            }
            Action::DeleteBranch => {
                if let Some(name) = self.repo.branches.get(self.selected[2]) {
                    self.mode = Mode::Confirm {
                        prompt: format!("delete branch {name}? y/n"),
                        action: ConfirmAction::DeleteBranch(name.clone()),
                    };
                }
            }
            Action::Push => self.pending_suspend = Some(svec(&["push"])),
            Action::Pull => self.pending_suspend = Some(svec(&["pull"])),
            Action::Fetch => self.pending_suspend = Some(svec(&["fetch"])),

            Action::ApplyStash => {
                let i = self.selected[4];
                if i < self.repo.stashes.len() {
                    self.write(svec(&["stash", "apply", &format!("stash@{{{i}}}")]));
                }
            }
            Action::PopStash => {
                let i = self.selected[4];
                if i < self.repo.stashes.len() {
                    self.write(svec(&["stash", "pop", &format!("stash@{{{i}}}")]));
                }
            }
            Action::DropStash => {
                let i = self.selected[4];
                if i < self.repo.stashes.len() {
                    self.mode = Mode::Confirm {
                        prompt: format!("drop stash@{{{i}}}? y/n"),
                        action: ConfirmAction::DropStash(i),
                    };
                }
            }
        }
    }

    fn apply_resp(&mut self, resp: Resp) {
        match resp {
            Resp::Status(files) => {
                self.repo.files = files;
                self.rebuild_tree();
            }
            Resp::Branches { current, names } => {
                self.repo.head = current;
                self.repo.branches = names;
                self.clamp(2);
            }
            Resp::Stashes(stashes) => {
                self.repo.stashes = stashes;
                self.clamp(4);
            }
            Resp::LogChunk { entries, done } => {
                self.repo.commits.extend(entries);
                self.repo.log_done = done;
                self.log_inflight = false;
                self.clamp(3);
            }
            // Ignore a diff for an old selection. Only the last request counts.
            Resp::Diff { seq, text } => {
                if seq == self.diff_seq {
                    self.repo.diff = text;
                    self.diff_scroll = 0;
                }
            }
            Resp::WriteDone { ok, msg } => {
                self.message = msg.clone();
                self.message_ok = ok;
                self.log_cmd(ok, msg);
                if ok {
                    self.refresh_all();
                }
            }
            Resp::Sync { ahead, behind, unpushed } => {
                self.repo.ahead = ahead;
                self.repo.behind = behind;
                self.repo.unpushed = unpushed;
            }
        }
    }

    fn clamp(&mut self, panel: usize) {
        let len = self.panel_len(panel);
        self.selected[panel] = self.selected[panel].min(len.saturating_sub(1));
    }

    // ponytail: a write triggers this and the fs watcher can trigger it
    // again. The refresh is idempotent and the watcher batches events.
    fn refresh_all(&mut self) {
        let Some(git) = &self.git else { return };
        for req in [Req::Status, Req::Branches, Req::Stashes, Req::Sync] {
            git.send(req);
        }
        self.repo.commits.clear();
        self.repo.log_done = false;
        git.send(Req::LogReset);
        git.send(Req::LogChunk { count: LOG_CHUNK });
        self.log_inflight = true;
        // Force a new diff request for the current selection.
        self.diff_target = None;
    }

    /// Send the requests that the new state makes necessary. The main loop
    /// calls this one time after each message burst. Thus a fast scroll makes
    /// one diff request, not one for each step.
    fn flush_requests(&mut self) {
        let Some(git) = &self.git else { return };

        // Load more commits before the selection comes near the loaded end.
        let near_end = self.selected[3] + LOG_CHUNK / 2 >= self.repo.commits.len();
        if self.focus == 3 && near_end && !self.repo.log_done && !self.log_inflight {
            git.send(Req::LogChunk { count: LOG_CHUNK });
            self.log_inflight = true;
        }

        let target = match self.focus {
            1 => self.selected_file().map(|f| DiffTarget::WorktreeFile(f.path.clone())),
            3 => self.repo.commits.get(self.selected[3]).map(|c| DiffTarget::Commit(c.id_str().to_string())),
            _ => None,
        };
        if target.is_some() && target != self.diff_target {
            self.diff_seq += 1;
            self.diff_target = target.clone();
            git.send(Req::Diff { seq: self.diff_seq, target: target.unwrap() });
        }
    }
}

fn svec(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    // Make an application with test data. The tests must not touch a real
    // repository.
    fn demo() -> App {
        let mut app = App::new();
        app.repo.head = Some("main".into());
        app.repo.ahead = 2;
        app.repo.files = [('M', true, "src/main.rs"), ('A', false, "src/app.rs"), ('?', false, "notes.txt")]
            .into_iter()
            .map(|(mark, staged, path)| FileEntry { mark, staged, path: path.into() })
            .collect();
        app.repo.unpushed = ["0a0c000".to_string(), "0a0c001".to_string()].into();
        app.repo.branches = vec!["feature/ui".into(), "main".into()];
        app.repo.stashes = vec!["stash@{0}: WIP on main".into()];
        app.repo.commits = (0..100_000)
            .map(|i| {
                let mut id = [b'0'; 7];
                id.copy_from_slice(format!("{:07x}", 0xa0c000 + i).as_bytes());
                let graph = ["●", "◉─╮", "● │", "│ ●", "●─╯"][i % 5];
                CommitEntry {
                    id,
                    graph: graph.into(),
                    subject: format!("fake: commit subject #{i}").into(),
                    author: "Test Author".into(),
                    time: 1_753_000_000 + i as u32,
                }
            })
            .collect();
        app.repo.diff = "diff --git a/src/main.rs b/src/main.rs\n+added line\n-removed line".into();
        app.rebuild_tree();
        app
    }

    fn draw(app: &App, width: u16, height: u16) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| ui::render(f, app)).unwrap();
        terminal
    }

    #[test]
    fn layout_80x24() {
        insta::assert_snapshot!(draw(&demo(), 80, 24).backend());
    }

    #[test]
    fn layout_200x50() {
        insta::assert_snapshot!(draw(&demo(), 200, 50).backend());
    }

    #[test]
    fn commits_panel_scrolled_deep() {
        // Put the selection far below one screen. This shows that the list
        // renders only the rows in view.
        let mut app = demo();
        app.focus = 3;
        app.selected[3] = 99_999;
        insta::assert_snapshot!(draw(&app, 80, 24).backend());
    }

    #[test]
    fn help_overlay() {
        let mut app = demo();
        app.mode = Mode::Help;
        insta::assert_snapshot!(draw(&app, 100, 30).backend());
    }

    #[test]
    fn zoomed_graph() {
        let mut app = demo();
        app.focus = 3;
        app.zoom = true;
        insta::assert_snapshot!(draw(&app, 80, 24).backend());
    }

    #[test]
    fn commit_prompt() {
        let mut app = demo();
        app.mode = Mode::Input { prompt: "commit message", buffer: "feat: x".into(), purpose: InputPurpose::CommitMsg };
        insta::assert_snapshot!(draw(&app, 80, 24).backend());
    }

    #[test]
    fn navigation() {
        let mut app = demo();
        app.apply(Action::Down);
        assert_eq!(app.selected[1], 1);
        app.apply(Action::Up);
        app.apply(Action::Up); // The selection stays at zero.
        assert_eq!(app.selected[1], 0);
        app.apply(Action::FocusPanel(3));
        assert_eq!(app.focus, 3);
        app.apply(Action::NextPanel);
        app.apply(Action::NextPanel); // The focus reaches the diff pane.
        assert_eq!(app.focus, 5);
        app.apply(Action::NextPanel); // The focus wraps to panel 0.
        assert_eq!(app.focus, 0);
        app.apply(Action::PrevPanel);
        assert_eq!(app.focus, 5);
    }
}
