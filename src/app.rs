use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{KeyCode, KeyEventKind};
use ratatui::crossterm::execute;
use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use std::collections::HashSet;

use crate::event::{self, Msg};
use crate::git::rebase::{self, RebaseInfo, TodoAction, TodoItem};
use crate::git::{
    self, BranchEntry, CommitEntry, DiffTarget, FileEntry, Git, Req, Resp, WorktreeEntry, patch,
};
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
    pub branches: Vec<BranchEntry>,
    pub commits: Vec<CommitEntry>,
    pub stashes: Vec<String>,
    pub log_done: bool,
    pub diff: String,
    /// The diff of the changes that are in the index.
    pub diff_staged: String,
    pub ahead: u32,
    pub behind: u32,
    pub bisecting: bool,
    /// Short ids of commits that the upstream branch does not have.
    pub unpushed: HashSet<String>,
}

pub enum InputPurpose {
    NewBranch,
    RenameBranch(String),
    StashMsg,
    /// Make a worktree. The text is the path of the new directory.
    AddWorktree,
    /// Run the text through the shell.
    Shell,
}

/// What the commit window does when the user sends it.
pub enum CommitPurpose {
    /// Make a new commit from the staged changes.
    New,
    /// Change the message of the commit at this position in the list.
    /// Position zero is HEAD, which needs only an amend.
    Reword(usize),
}

pub struct LogEntry {
    pub ok: bool,
    pub cmd: String,
    pub ms: u64,
    /// What the command printed. The log is the only place that shows it.
    pub output: Vec<String>,
}

pub enum ConfirmAction {
    DeleteBranch { name: String, force: bool },
    DropStash(usize),
    Merge(String),
    Revert(String),
    RemoveWorktree(String),
    /// Go to the worktree that holds a branch.
    GoToWorktree(String),
    /// Run each command in order. Discard and delete need two commands,
    /// because tracked files and new files need different treatment.
    RunAll(Vec<Vec<String>>),
}

pub enum Mode {
    Normal,
    Input { prompt: &'static str, buffer: String, purpose: InputPurpose },
    Confirm { prompt: String, action: ConfirmAction },
    /// The hunk view of one file. `cursor` is the hunk in view. `line` is
    /// the body line in that hunk. `picked` holds the marked body lines.
    Hunks {
        path: String,
        header: String,
        hunks: Vec<String>,
        cursor: usize,
        line: usize,
        picked: Vec<usize>,
    },
    Help,
    /// The worktree list. It opens over the panels.
    Worktrees { list: Vec<WorktreeEntry>, cursor: usize },
    /// The commit message window. It has a summary line and a body.
    CommitMsg { summary: String, body: String, on_body: bool, purpose: CommitPurpose },
    /// The todo list editor of an interactive rebase. `base` is the commit
    /// that the rebase starts from. None means the rebase starts at the root.
    Rebase { items: Vec<TodoItem>, cursor: usize, base: Option<String> },
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
    /// The number of lines in the diff pane. The scroll stops there.
    diff_lines: u16,
    pub tree: Vec<TreeRow>,
    pub collapsed: HashSet<String>,
    pub cmd_log: Vec<LogEntry>,
    pub show_log: bool,
    /// The commands that run now. The bar shows them.
    pub running: Vec<String>,
    /// Counts up while a command runs. It moves the spinner.
    tick: usize,
    /// The size of the terminal at the last draw. The mouse needs it to
    /// know which panel is under the pointer.
    area: ratatui::layout::Rect,
    pub rebase: Option<RebaseInfo>,
    git: Option<Git>,
    log_inflight: bool,
    diff_seq: u64,
    diff_target: Option<DiffTarget>,
    pending_suspend: Option<(Vec<String>, Vec<(String, String)>)>,
    pause: Arc<AtomicBool>,
    /// A copy of the message sender. A move to another worktree needs it to
    /// start new workers.
    tx: Option<mpsc::Sender<Msg>>,
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
            diff_lines: 0,
            tree: Vec::new(),
            collapsed: HashSet::new(),
            cmd_log: Vec::new(),
            show_log: true,
            running: Vec::new(),
            tick: 0,
            area: ratatui::layout::Rect::ZERO,
            rebase: None,
            git: None,
            log_inflight: false,
            diff_seq: 0,
            diff_target: None,
            pending_suspend: None,
            pause: Arc::new(AtomicBool::new(false)),
            tx: None,
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        event::spawn_input(tx.clone(), self.pause.clone());
        let git = git::spawn(tx.clone())?;
        git::watch::spawn(git.git_dir.clone(), tx.clone());
        self.git = Some(git);
        self.tx = Some(tx);
        self.refresh_all();

        // The mouse moves the focus and the selection.
        execute!(std::io::stdout(), EnableMouseCapture)?;
        while !self.quit {
            // Ask the backend for the size. get_frame would take a Frame
            // out of the draw cycle and stop the next draw from showing.
            let size = terminal.size()?;
            self.area = ratatui::layout::Rect::new(0, 0, size.width, size.height);
            terminal.draw(|f| ui::render(f, self))?;
            // The loop waits for a message and uses no processor time. While
            // a command runs, it wakes often enough to move the spinner.
            let msg = if self.running.is_empty() {
                rx.recv()?
            } else {
                match rx.recv_timeout(Duration::from_millis(90)) {
                    Ok(msg) => msg,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        self.tick += 1;
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            };
            // Drain the queue before each draw. This makes one draw for a
            // burst of messages, not one draw for each message.
            self.update(msg);
            while let Ok(msg) = rx.try_recv() {
                self.update(msg);
            }
            self.flush_requests();
            if let Some((args, envs)) = self.pending_suspend.take() {
                self.suspend_and_run(terminal, args, envs)?;
            }
        }
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
        Ok(())
    }

    /// Give the terminal to a git child process, for example push or an
    /// editor for a commit message. Restore the terminal after it.
    fn suspend_and_run(
        &mut self,
        terminal: &mut DefaultTerminal,
        args: Vec<String>,
        envs: Vec<(String, String)>,
    ) -> Result<()> {
        let Some(git) = &self.git else { return Ok(()) };
        self.pause.store(true, Ordering::Relaxed);
        let start = std::time::Instant::now();
        disable_raw_mode()?;
        // The child program owns the terminal, thus it must own the mouse.
        execute!(std::io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&git.root)
            .args(&args)
            .envs(envs)
            .status();
        enable_raw_mode()?;
        execute!(std::io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        terminal.clear()?;
        self.pause.store(false, Ordering::Relaxed);
        let ok = matches!(&status, Ok(s) if s.success());
        let err = match status {
            Ok(s) if s.success() => None,
            Ok(s) => Some(s.to_string()),
            Err(e) => Some(e.to_string()),
        };
        let ms = start.elapsed().as_millis() as u64;
        self.log_cmd(ok, format!("git {}", args.join(" ")), ms, err.into_iter().collect());
        self.refresh_all();
        Ok(())
    }

    pub fn update(&mut self, msg: Msg) {
        match msg {
            Msg::Key(key) if key.kind == KeyEventKind::Press => self.handle_key(key),
            Msg::Mouse(m) => self.handle_mouse(m),
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
                // While a rebase is stopped, these keys come first. They
                // replace the normal meaning of the key.
                if self.rebase.is_some() {
                    match key.code {
                        KeyCode::Char('c') => return self.apply(Action::RebaseContinue),
                        KeyCode::Char('s') => return self.apply(Action::RebaseSkip),
                        KeyCode::Char('A') => return self.apply(Action::RebaseAbort),
                        _ => {}
                    }
                }
                if let Some(action) = action_for(key, self.focus) {
                    self.apply(action);
                }
            }
            Mode::Help => self.mode = Mode::Normal,
            Mode::Worktrees { list, cursor } => match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('W') => self.mode = Mode::Normal,
                KeyCode::Char('j') | KeyCode::Down => {
                    *cursor = (*cursor + 1).min(list.len().saturating_sub(1))
                }
                KeyCode::Char('k') | KeyCode::Up => *cursor = cursor.saturating_sub(1),
                KeyCode::Char('n') => {
                    self.mode = Mode::Input {
                        prompt: "path for the new worktree",
                        buffer: String::new(),
                        purpose: InputPurpose::AddWorktree,
                    };
                }
                KeyCode::Char('d') => {
                    let Some(w) = list.get(*cursor) else { return };
                    if w.current {
                        self.mode = Mode::Normal;
                        self.message = "cannot remove the worktree you are in".into();
                        self.message_ok = false;
                        return;
                    }
                    let path = w.path.clone();
                    self.mode = Mode::Confirm {
                        prompt: format!("remove the worktree {path}?"),
                        action: ConfirmAction::RemoveWorktree(path),
                    };
                }
                KeyCode::Enter => {
                    let Some(w) = list.get(*cursor) else { return };
                    let path = w.path.clone();
                    self.mode = Mode::Normal;
                    self.open_worktree(path);
                }
                _ => {}
            },
            Mode::CommitMsg { summary, body, on_body, .. } => match key.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                // Tab moves between the summary line and the body.
                KeyCode::Tab | KeyCode::BackTab => *on_body = !*on_body,
                KeyCode::Down if !*on_body => *on_body = true,
                KeyCode::Up if *on_body => *on_body = false,
                KeyCode::Backspace => {
                    if *on_body { body.pop() } else { summary.pop() };
                }
                // The enter key makes a new line in the body. In the summary
                // line it sends the commit.
                KeyCode::Enter if *on_body => body.push('\n'),
                KeyCode::Enter => {
                    let Mode::CommitMsg { summary, body, purpose, .. } =
                        std::mem::replace(&mut self.mode, Mode::Normal)
                    else {
                        return;
                    };
                    self.submit_commit(summary, body, purpose);
                }
                KeyCode::Char(c) => {
                    if *on_body { body.push(c) } else { summary.push(c) }
                }
                _ => {}
            },
            Mode::Rebase { items, cursor, .. } => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.mode = Mode::Normal,
                KeyCode::Char('j') | KeyCode::Down => *cursor = (*cursor + 1).min(items.len() - 1),
                KeyCode::Char('k') | KeyCode::Up => *cursor = cursor.saturating_sub(1),
                // Control with j or k moves the commit itself.
                KeyCode::Char('J') if *cursor + 1 < items.len() => {
                    items.swap(*cursor, *cursor + 1);
                    *cursor += 1;
                }
                KeyCode::Char('K') if *cursor > 0 => {
                    items.swap(*cursor, *cursor - 1);
                    *cursor -= 1;
                }
                KeyCode::Char(c @ ('p' | 'r' | 'e' | 's' | 'f' | 'd')) => {
                    items[*cursor].action = match c {
                        'p' => TodoAction::Pick,
                        'r' => TodoAction::Reword,
                        'e' => TodoAction::Edit,
                        's' => TodoAction::Squash,
                        'f' => TodoAction::Fixup,
                        _ => TodoAction::Drop,
                    };
                }
                KeyCode::Enter => {
                    let Mode::Rebase { items, base, .. } =
                        std::mem::replace(&mut self.mode, Mode::Normal)
                    else {
                        return;
                    };
                    self.run_rebase(items, base);
                }
                _ => {}
            },
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
                    match action {
                        ConfirmAction::RunAll(cmds) => {
                            for cmd in cmds {
                                self.write(cmd);
                            }
                            return;
                        }
                        ConfirmAction::GoToWorktree(path) => return self.open_worktree(path),
                        _ => {}
                    }
                    let args: Vec<String> = match action {
                        // A plain delete refuses a branch that is not
                        // merged. The force delete does not refuse.
                        ConfirmAction::DeleteBranch { name, force } => {
                            svec(&["branch", if force { "-D" } else { "-d" }, &name])
                        }
                        ConfirmAction::DropStash(i) => svec(&["stash", "drop", &format!("stash@{{{i}}}")]),
                        ConfirmAction::Merge(name) => svec(&["merge", "--no-edit", &name]),
                        ConfirmAction::Revert(id) => svec(&["revert", "--no-edit", &id]),
                        ConfirmAction::RemoveWorktree(path) => {
                            svec(&["worktree", "remove", &path])
                        }
                        ConfirmAction::RunAll(_) | ConfirmAction::GoToWorktree(_) => return,
                    };
                    self.write(args);
                }
                _ => self.mode = Mode::Normal,
            },
            Mode::Hunks { header, hunks, cursor, line, picked, .. } => {
                let body_len = hunks[*cursor].lines().count().saturating_sub(1);
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => self.mode = Mode::Normal,
                    KeyCode::Char('j') | KeyCode::Down => {
                        *line = (*line + 1).min(body_len.saturating_sub(1))
                    }
                    KeyCode::Char('k') | KeyCode::Up => *line = line.saturating_sub(1),
                    // A move to another hunk drops the marked lines. They
                    // point at the old hunk.
                    KeyCode::Char('J') | KeyCode::Tab => {
                        *cursor = (*cursor + 1).min(hunks.len() - 1);
                        *line = 0;
                        picked.clear();
                    }
                    KeyCode::Char('K') | KeyCode::BackTab => {
                        *cursor = cursor.saturating_sub(1);
                        *line = 0;
                        picked.clear();
                    }
                    KeyCode::Char(' ') => {
                        match picked.iter().position(|p| p == line) {
                            Some(i) => {
                                picked.remove(i);
                            }
                            None => picked.push(*line),
                        }
                        *line = (*line + 1).min(body_len.saturating_sub(1));
                    }
                    // Stage the whole hunk.
                    KeyCode::Char('a') => {
                        let patch = patch::hunk_patch(header, &hunks[*cursor]);
                        self.apply_and_leave(patch);
                    }
                    // Stage the marked lines only.
                    KeyCode::Enter => {
                        let marks = if picked.is_empty() { vec![*line] } else { picked.clone() };
                        match patch::subset_hunk(&hunks[*cursor], &marks) {
                            Some(sub) => {
                                let patch = patch::hunk_patch(header, &sub);
                                self.apply_and_leave(patch);
                            }
                            None => {
                                self.message = "mark a line that adds or removes text".into();
                                self.message_ok = false;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn submit_input(&mut self, purpose: InputPurpose, buffer: String) {
        if buffer.is_empty() {
            return;
        }
        if let InputPurpose::Shell = purpose {
            self.running.push(format!(": {buffer}"));
            if let Some(git) = &self.git {
                git.send(Req::Shell(buffer));
            }
            return;
        }
        let args: Vec<String> = match purpose {
            InputPurpose::NewBranch => svec(&["checkout", "-b", &buffer]),
            InputPurpose::RenameBranch(old) => svec(&["branch", "-m", &old, &buffer]),
            InputPurpose::StashMsg => svec(&["stash", "push", "-m", &buffer]),
            // A new worktree needs a branch. Take the name of the last part
            // of the path.
            InputPurpose::AddWorktree => {
                let name = buffer.rsplit('/').next().unwrap_or("work").to_string();
                svec(&["worktree", "add", "-b", &name, &buffer])
            }
            InputPurpose::Shell => return,
        };
        self.write(args);
    }

    /// Commit with the summary and the body. Git puts an empty line between
    /// two message parts, thus the body becomes a real commit body.
    fn submit_commit(&mut self, summary: String, body: String, purpose: CommitPurpose) {
        if summary.trim().is_empty() {
            self.message = "the summary must have text".into();
            self.message_ok = false;
            return;
        }
        let msg_args = |verb: &str| {
            let mut args = svec(&[verb, "-m", &summary]);
            if !body.trim().is_empty() {
                args.push("-m".into());
                args.push(body.clone());
            }
            args
        };
        match purpose {
            CommitPurpose::New => self.write(msg_args("commit")),
            // HEAD needs only an amend. No rebase runs.
            CommitPurpose::Reword(0) => {
                let mut args = msg_args("commit");
                args.insert(1, "--amend".into());
                self.write(args);
            }
            // An older commit needs a rebase with one reword step. Git asks
            // this program for both the todo list and the new message.
            CommitPurpose::Reword(index) => {
                let Some(git) = &self.git else { return };
                let mut text = summary.clone();
                if !body.trim().is_empty() {
                    text.push_str("\n\n");
                    text.push_str(body.trim_end());
                }
                text.push('\n');
                let msg_path = git.git_dir.join("lazier-msg");
                if let Err(e) = std::fs::write(&msg_path, text) {
                    self.message = e.to_string();
                    self.message_ok = false;
                    return;
                }
                let Some((mut items, base)) = self.rebase_slice(index) else { return };
                // The target is the oldest commit in the list.
                items[index].action = TodoAction::Reword;
                let editor = self.seq_editor_cmd(&msg_path);
                self.run_rebase_with(items, base, vec![("GIT_EDITOR".into(), editor)]);
            }
        }
    }

    /// Move the program to another worktree. The old workers stop when
    /// their sender goes away. New workers start at the new directory.
    fn open_worktree(&mut self, path: String) {
        if let Err(e) = std::env::set_current_dir(&path) {
            self.message = e.to_string();
            self.message_ok = false;
            return;
        }
        let Some(tx) = self.tx.clone() else { return };
        match git::spawn(tx.clone()) {
            Ok(git) => {
                git::watch::spawn(git.git_dir.clone(), tx);
                self.git = Some(git);
                self.repo = RepoState::default();
                self.selected = [0; 6];
                self.tree.clear();
                self.refresh_all();
                self.log_cmd(true, format!("open worktree {path}"), 0, Vec::new());
            }
            Err(e) => {
                self.message = e.to_string();
                self.message_ok = false;
            }
        }
    }

    // Send the patch, then leave the hunk view. The line numbers of the
    // other hunks change after the apply, thus they must not stay in use.
    fn apply_and_leave(&mut self, patch: String) {
        self.mode = Mode::Normal;
        if let Some(git) = &self.git {
            git.send(Req::ApplyPatch { patch, reverse: false });
        }
    }

    /// Build the todo items for the commits from HEAD down to `index`, and
    /// the commit that the rebase starts from.
    fn rebase_slice(&mut self, index: usize) -> Option<(Vec<TodoItem>, Option<String>)> {
        let last = self.repo.commits.get(index)?;
        let base = if index + 1 < self.repo.commits.len() {
            Some(format!("{}^", last.id_str()))
        } else if self.repo.log_done {
            None // The oldest commit is a root commit.
        } else {
            self.message = "load more commits first".into();
            self.message_ok = false;
            return None;
        };
        let items = self.repo.commits[..=index]
            .iter()
            .map(|c| TodoItem {
                action: TodoAction::Pick,
                id: c.id_str().to_string(),
                subject: c.subject.to_string(),
            })
            .collect();
        Some((items, base))
    }

    /// The command line that makes git call this program as an editor. The
    /// program copies `file` over the file that git wants edited.
    fn seq_editor_cmd(&self, file: &std::path::Path) -> String {
        let exe = std::env::current_exe().unwrap_or_default();
        format!(
            "{} --seq-editor {}",
            rebase::sh_quote(&exe.to_string_lossy()),
            rebase::sh_quote(&file.to_string_lossy())
        )
    }

    fn write(&mut self, args: Vec<String>) {
        // A slow command shows in the bar until it ends.
        if git::is_network(&args) {
            self.running.push(format!("git {}", args.join(" ")));
        }
        if let Some(git) = &self.git {
            git.send(Req::Write(args));
        }
    }

    /// Run a git command with the real terminal. Use it for a command that
    /// asks the user something, for example a password or a commit message.
    fn suspend(&mut self, args: Vec<String>) {
        self.pending_suspend = Some((args, Vec::new()));
    }

    /// Open the todo editor for the commits above the selected one. The
    /// selected commit is the oldest commit in the list.
    fn start_rebase(&mut self) {
        if let Some((items, base)) = self.rebase_slice(self.selected[3]) {
            self.mode = Mode::Rebase { items, cursor: 0, base };
        }
    }

    /// Write the todo list, then run the rebase with the real terminal.
    /// Git calls this program as the sequence editor, thus git shows no
    /// editor for the todo list. A reword step still opens the user editor.
    fn run_rebase(&mut self, items: Vec<TodoItem>, base: Option<String>) {
        self.run_rebase_with(items, base, Vec::new());
    }

    fn run_rebase_with(
        &mut self,
        items: Vec<TodoItem>,
        base: Option<String>,
        mut envs: Vec<(String, String)>,
    ) {
        let Some(git) = &self.git else { return };
        let todo_path = git.git_dir.join("lazier-todo");
        if let Err(e) = std::fs::write(&todo_path, rebase::serialize(&items)) {
            self.message = e.to_string();
            self.message_ok = false;
            return;
        }
        envs.push(("GIT_SEQUENCE_EDITOR".into(), self.seq_editor_cmd(&todo_path)));
        let args = match &base {
            Some(b) => svec(&["rebase", "-i", b]),
            None => svec(&["rebase", "-i", "--root"]),
        };
        self.pending_suspend = Some((args, envs));
    }

    fn log_cmd(&mut self, ok: bool, cmd: String, ms: u64, output: Vec<String>) {
        self.cmd_log.push(LogEntry { ok, cmd, ms, output });
        // Keep the log short. Old entries have no value.
        if self.cmd_log.len() > 100 {
            self.cmd_log.remove(0);
        }
    }

    /// The mouse moves the focus and the selection. A window is open in
    /// every mode but Normal, thus the mouse does nothing then.
    fn handle_mouse(&mut self, m: ratatui::crossterm::event::MouseEvent) {
        use ratatui::crossterm::event::{MouseButton, MouseEventKind};
        if !matches!(self.mode, Mode::Normal) || self.zoom {
            return;
        }
        let p = ui::panes(self.area, self.show_log);
        // Find the panel under the pointer. Panel five is the diff.
        let hit = p
            .left
            .iter()
            .position(|r| contains(*r, m.column, m.row))
            .or_else(|| contains(p.diff, m.column, m.row).then_some(5));
        let Some(panel) = hit else { return };

        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.message.clear();
                self.focus = panel;
                if panel == 5 {
                    return;
                }
                // The row under the pointer becomes the selection.
                let area = p.left[panel];
                let visible = area.height.saturating_sub(2) as usize;
                let len = self.panel_len(panel);
                let inside = m.row.saturating_sub(area.y + 1) as usize;
                let idx = ui::list_offset(self.selected[panel], len, visible) + inside;
                if inside < visible && idx < len {
                    self.selected[panel] = idx;
                }
            }
            MouseEventKind::ScrollDown => {
                self.focus = panel;
                self.apply(Action::Down);
            }
            MouseEventKind::ScrollUp => {
                self.focus = panel;
                self.apply(Action::Up);
            }
            _ => {}
        }
    }

    /// The spinner character for this moment. None when nothing runs.
    pub fn spinner(&self) -> Option<char> {
        const FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        (!self.running.is_empty()).then(|| FRAMES[self.tick % FRAMES.len()])
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

    /// The path that a file action works on. It gives the pathspec, whether
    /// git knows the path, and a name to show the user. The root row gives
    /// the whole work tree.
    fn file_target(&self) -> Option<(String, bool, String)> {
        let row = self.selected_row()?;
        if let Some(dir) = &row.dir {
            let path = if dir.is_empty() { ".".to_string() } else { dir.clone() };
            let label = if dir.is_empty() { "the whole work tree".into() } else { format!("{dir}/") };
            // A directory can hold both kinds of file.
            return Some((path, false, label));
        }
        let f = self.repo.files.get(row.file?)?;
        Some((f.path.clone(), f.work == '?', f.path.clone()))
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
            Action::Down if self.focus == 5 => self.scroll_diff(1),
            Action::Up if self.focus == 5 => self.scroll_diff(-1),
            Action::PageDown if self.focus == 5 => self.scroll_diff(15),
            Action::PageUp if self.focus == 5 => self.scroll_diff(-15),
            Action::Top if self.focus == 5 => self.diff_scroll = 0,
            Action::Bottom if self.focus == 5 => self.scroll_diff(i16::MAX),
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
            Action::DiffScroll(delta) => self.scroll_diff(delta as i16),
            Action::ZoomGraph => self.zoom = !self.zoom,
            Action::Help => self.mode = Mode::Help,
            Action::ToggleLog => self.show_log = !self.show_log,
            Action::Refresh => self.refresh_all(),

            Action::ToggleStage => {
                // On a directory row, stage the whole directory. The root
                // row has an empty path, which is not a valid pathspec.
                if let Some(dir) = self.selected_row().and_then(|r| r.dir.clone()) {
                    let args = if dir.is_empty() {
                        svec(&["add", "-A"])
                    } else {
                        svec(&["add", "--", &dir])
                    };
                    self.write(args);
                } else if let Some(f) = self.selected_file() {
                    let args = if f.staged() && f.work == ' ' {
                        svec(&["restore", "--staged", "--", &f.path])
                    } else {
                        svec(&["add", "--", &f.path])
                    };
                    self.write(args);
                }
            }
            Action::StageAll => self.write(svec(&["add", "-A"])),
            Action::CommitPrompt => {
                self.mode = Mode::CommitMsg {
                    summary: String::new(),
                    body: String::new(),
                    on_body: false,
                    purpose: CommitPurpose::New,
                };
            }
            Action::CommitEditor => self.suspend(svec(&["commit"])),
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
                        self.mode = Mode::Hunks {
                            path: f.path.clone(),
                            header,
                            hunks,
                            cursor: 0,
                            line: 0,
                            picked: Vec::new(),
                        };
                    }
                    _ => self.message = "no hunks in this file".into(),
                }
            }
            // Discard removes work that has no commit. It always asks first.
            Action::DiscardChanges => {
                let Some((target, untracked, label)) = self.file_target() else { return };
                let mut cmds = Vec::new();
                if !untracked {
                    cmds.push(svec(&["restore", "--staged", "--worktree", "--", &target]));
                }
                // New files have no old state. Only a delete removes them.
                cmds.push(svec(&["clean", "-fd", "--", &target]));
                self.mode = Mode::Confirm {
                    prompt: format!("discard all changes in {label}? this cannot be undone."),
                    action: ConfirmAction::RunAll(cmds),
                };
            }
            Action::DeleteFile => {
                let Some((target, untracked, label)) = self.file_target() else { return };
                // A delete of the root would remove the whole work tree.
                if target == "." {
                    self.message = "select a file or a directory, not the root".into();
                    self.message_ok = false;
                    return;
                }
                let cmds = if untracked {
                    vec![svec(&["clean", "-fd", "--", &target])]
                } else {
                    vec![svec(&["rm", "-r", "-f", "--", &target])]
                };
                self.mode = Mode::Confirm {
                    prompt: format!("delete {label} from the disk?"),
                    action: ConfirmAction::RunAll(cmds),
                };
            }

            Action::TakeOurs | Action::TakeTheirs => {
                let side = if matches!(action, Action::TakeOurs) { "--ours" } else { "--theirs" };
                if let Some(f) = self.selected_file()
                    && f.conflicted()
                {
                    let path = f.path.clone();
                    self.write(svec(&["checkout", side, "--", &path]));
                    self.write(svec(&["add", "--", &path]));
                }
            }

            Action::Checkout => {
                let Some(b) = self.repo.branches.get(self.selected[2]) else { return };
                // A checkout of the branch you are on does nothing, but it
                // still reads every file. Do not run it.
                if b.current {
                    self.message = format!("you are already on {}", b.name);
                    self.message_ok = true;
                    return;
                }
                let name = b.name.clone();
                self.write(svec(&["checkout", &name]));
            }
            Action::NewBranchPrompt => {
                self.mode = Mode::Input { prompt: "new branch name", buffer: String::new(), purpose: InputPurpose::NewBranch };
            }
            Action::DeleteBranch { force } => {
                let Some(b) = self.repo.branches.get(self.selected[2]) else { return };
                if b.current {
                    self.message = "cannot delete the branch you are on".into();
                    self.message_ok = false;
                    return;
                }
                let word = if force { "force delete" } else { "delete" };
                self.mode = Mode::Confirm {
                    prompt: format!("{word} branch {}?", b.name),
                    action: ConfirmAction::DeleteBranch { name: b.name.clone(), force },
                };
            }
            Action::RenameBranchPrompt => {
                if let Some(b) = self.repo.branches.get(self.selected[2]) {
                    self.mode = Mode::Input {
                        prompt: "new name for the branch",
                        buffer: b.name.clone(),
                        purpose: InputPurpose::RenameBranch(b.name.clone()),
                    };
                }
            }
            Action::MergeBranch => {
                let Some(b) = self.repo.branches.get(self.selected[2]) else { return };
                if b.current {
                    self.message = "cannot merge a branch into itself".into();
                    self.message_ok = false;
                    return;
                }
                let name = b.name.clone();
                let head = self.repo.head.clone().unwrap_or_else(|| "HEAD".into());
                self.mode = Mode::Confirm {
                    prompt: format!("merge {name} into {head}?"),
                    action: ConfirmAction::Merge(name),
                };
            }

            Action::CherryPick => {
                if let Some(c) = self.repo.commits.get(self.selected[3]) {
                    let id = c.id_str().to_string();
                    self.write(svec(&["cherry-pick", "--no-commit", &id]));
                }
            }
            Action::BisectBad | Action::BisectGood | Action::BisectSkip | Action::BisectReset => {
                let id = self.repo.commits.get(self.selected[3]).map(|c| c.id_str().to_string());
                if let Some(args) = bisect_command(self.repo.bisecting, &action, id.as_deref()) {
                    self.write(args);
                }
            }

            Action::Worktrees => {
                if let Some(git) = &self.git {
                    git.send(Req::Worktrees);
                }
            }

            Action::RewordCommit => {
                // Read the old message first. The window opens when it
                // arrives.
                let index = self.selected[3];
                if let Some(c) = self.repo.commits.get(index)
                    && let Some(git) = &self.git
                {
                    git.send(Req::ReadMessage { id: c.id_str().to_string(), index });
                }
            }
            Action::RevertCommit => {
                if let Some(c) = self.repo.commits.get(self.selected[3]) {
                    let id = c.id_str().to_string();
                    let subject = c.subject.to_string();
                    self.mode = Mode::Confirm {
                        prompt: format!("revert {id} \"{subject}\"?"),
                        action: ConfirmAction::Revert(id),
                    };
                }
            }
            // These run in the background. The bar says one is running.
            Action::Push => self.write(svec(&["push"])),
            Action::Pull => self.write(svec(&["pull"])),
            Action::Fetch => self.write(svec(&["fetch"])),
            Action::ShellPrompt => {
                self.mode = Mode::Input {
                    prompt: "shell command",
                    buffer: String::new(),
                    purpose: InputPurpose::Shell,
                };
            }

            Action::InteractiveRebase => self.start_rebase(),
            // These three keys work only while a rebase is stopped.
            Action::RebaseContinue => self.suspend(svec(&["rebase", "--continue"])),
            Action::RebaseSkip => self.suspend(svec(&["rebase", "--skip"])),
            Action::RebaseAbort => self.write(svec(&["rebase", "--abort"])),

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
                        prompt: format!("drop stash@{{{i}}}?"),
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
            Resp::Branches { current, entries } => {
                self.repo.head = current;
                self.repo.branches = entries;
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
            // HEAD moved, thus the old list is wrong.
            Resp::LogReplace { entries, done } => {
                self.repo.commits = entries;
                self.repo.log_done = done;
                self.log_inflight = false;
                self.clamp(3);
            }
            // Ignore a diff for an old selection. Only the last request counts.
            Resp::Diff { seq, text, staged } => {
                if seq == self.diff_seq {
                    // Both diffs share one pane, thus the scroll counts the
                    // lines of both and the two header rows.
                    let total = text.lines().count() + staged.lines().count() + 2;
                    self.diff_lines = total.min(u16::MAX as usize) as u16;
                    self.repo.diff = text;
                    self.repo.diff_staged = staged;
                    self.diff_scroll = 0;
                }
            }
            Resp::WriteDone { ok, cmd, output, ms } => {
                // The command log holds the result and the output. The bar
                // keeps its key hints, thus a command never hides them.
                if let Some(i) = self.running.iter().position(|c| *c == cmd) {
                    self.running.remove(i);
                }
                // A checkout fails when another worktree holds the branch.
                // Offer to go to that worktree instead.
                if !ok
                    && let Some(path) = worktree_in_use(&output)
                {
                    self.mode = Mode::Confirm {
                        prompt: format!("that branch is checked out at {path}. go there?"),
                        action: ConfirmAction::GoToWorktree(path),
                    };
                }
                self.log_cmd(ok, cmd, ms, output);
                if ok {
                    self.refresh_all();
                }
            }
            // The reword window opens when the old message arrives.
            Resp::Message { text, index } => {
                let (summary, body) = match text.split_once('\n') {
                    Some((s, b)) => (s.to_string(), b.trim_start_matches('\n').to_string()),
                    None => (text, String::new()),
                };
                self.mode = Mode::CommitMsg {
                    summary,
                    body,
                    on_body: false,
                    purpose: CommitPurpose::Reword(index),
                };
            }
            Resp::Sync { ahead, behind, unpushed } => {
                self.repo.ahead = ahead;
                self.repo.behind = behind;
                self.repo.unpushed = unpushed;
            }
            Resp::Worktrees(list) => self.mode = Mode::Worktrees { list, cursor: 0 },
        }
    }

    // Keep at least one line of the diff in view.
    fn scroll_diff(&mut self, delta: i16) {
        let last = self.diff_lines.saturating_sub(1);
        self.diff_scroll = self.diff_scroll.saturating_add_signed(delta).min(last);
    }

    fn clamp(&mut self, panel: usize) {
        let len = self.panel_len(panel);
        self.selected[panel] = self.selected[panel].min(len.saturating_sub(1));
    }

    fn refresh_all(&mut self) {
        let Some(git) = &self.git else { return };
        for req in [Req::Status, Req::Branches, Req::Stashes, Req::Sync] {
            git.send(req);
        }
        // The log thread walks again only when HEAD moved. A stage or a
        // fetch keeps the list that is already in memory.
        git.send(Req::LogRefresh { count: LOG_CHUNK });
        // Force a new diff request for the current selection.
        self.diff_target = None;
        let dir = self.git.as_ref().map(|g| g.git_dir.clone());
        self.rebase = dir.as_deref().and_then(rebase::detect);
        self.repo.bisecting = dir.as_deref().is_some_and(rebase::bisecting);
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

// True when the point sits inside the rectangle.
fn contains(r: ratatui::layout::Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
}

fn svec(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

/// Find the worktree path in a git error. Git says:
///   fatal: 'x' is already used by worktree at '/path/to/tree'
fn worktree_in_use(output: &[String]) -> Option<String> {
    for line in output {
        if let Some((_, rest)) = line.split_once("is already used by worktree at ") {
            let path = rest.trim().trim_matches('\'').trim_matches('"');
            if !path.is_empty() {
                return Some(path.to_string());
            }
        }
    }
    None
}

/// The git arguments for a bisect key. None means the key does nothing.
/// Before a bisect runs, only the good and the bad key can start one.
fn bisect_command(bisecting: bool, action: &Action, id: Option<&str>) -> Option<Vec<String>> {
    Some(match (action, bisecting) {
        (Action::BisectBad, true) => svec(&["bisect", "bad"]),
        (Action::BisectGood, true) => svec(&["bisect", "good"]),
        (Action::BisectSkip, true) => svec(&["bisect", "skip"]),
        (Action::BisectReset, true) => svec(&["bisect", "reset"]),
        // The bad commit starts the bisect. Git then needs a good one.
        (Action::BisectBad, false) => svec(&["bisect", "start", id?]),
        (Action::BisectGood, false) => svec(&["bisect", "good", id?]),
        _ => return None,
    })
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
        app.repo.files = [('M', 'M', "src/main.rs"), (' ', 'A', "src/app.rs"), (' ', '?', "notes.txt")]
            .into_iter()
            .map(|(index, work, path)| FileEntry { index, work, path: path.into() })
            .collect();
        app.repo.unpushed = ["0a0c000".to_string(), "0a0c001".to_string()].into();
        app.repo.branches =
            [("main", true, 2, 0, "2h"), ("feature/ui", false, 1, 3, "1d"), ("old/thing", false, 0, 0, "3w")]
                .into_iter()
                .map(|(name, current, ahead, behind, age)| BranchEntry {
                    name: name.into(),
                    current,
                    ahead,
                    behind,
                    gone: false,
                    age: age.into(),
                })
                .collect();
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
        app.repo.diff_staged = "diff --git a/src/main.rs b/src/main.rs\n+staged line".into();
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
    fn rebase_editor() {
        let mut app = demo();
        app.focus = 3;
        app.selected[3] = 3;
        app.start_rebase();
        // Give the four commits different actions.
        if let Mode::Rebase { items, cursor, .. } = &mut app.mode {
            items[1].action = TodoAction::Squash;
            items[2].action = TodoAction::Drop;
            items[3].action = TodoAction::Reword;
            *cursor = 2;
        } else {
            panic!("the rebase editor did not open");
        }
        insta::assert_snapshot!(draw(&app, 100, 24).backend());
    }

    #[test]
    fn rebase_todo_matches_editor_order() {
        let mut app = demo();
        app.selected[3] = 2;
        app.start_rebase();
        let Mode::Rebase { items, base, .. } = &app.mode else { panic!("no editor") };
        // The base is the parent of the oldest commit in the list.
        assert_eq!(base.as_deref(), Some("0a0c002^"));
        // The file starts with the oldest commit.
        assert!(rebase::serialize(items).starts_with("pick 0a0c002"));
    }

    #[test]
    fn commit_window() {
        let mut app = demo();
        app.mode = Mode::CommitMsg {
            summary: "feat: add the commit window".into(),
            body: "The window has a summary line and a body.".into(),
            on_body: true,
            purpose: CommitPurpose::New,
        };
        insta::assert_snapshot!(draw(&app, 100, 30).backend());
    }

    #[test]
    fn reword_window() {
        let mut app = demo();
        app.mode = Mode::CommitMsg {
            summary: "fix: the old summary".into(),
            body: String::new(),
            on_body: false,
            purpose: CommitPurpose::Reword(0),
        };
        insta::assert_snapshot!(draw(&app, 100, 30).backend());
    }

    #[test]
    fn empty_summary_is_refused() {
        let mut app = demo();
        app.submit_commit(String::new(), "body only".into(), CommitPurpose::New);
        assert!(!app.message_ok);
        // Nothing went to git.
        assert!(app.cmd_log.is_empty());
    }

    #[test]
    fn the_current_branch_cannot_be_deleted() {
        let mut app = demo();
        app.focus = 2;
        app.selected[2] = 0; // The demo puts the current branch first.
        assert!(app.repo.branches[0].current);
        app.apply(Action::DeleteBranch { force: false });
        assert!(!app.message_ok);
        assert!(matches!(app.mode, Mode::Normal), "no confirm window opens");
    }

    #[test]
    fn command_log_keeps_the_result_and_the_time() {
        let mut app = demo();
        app.apply_resp(Resp::WriteDone {
            ok: false,
            cmd: "git branch -d old".into(),
            output: vec!["error: the branch is not merged".into()],
            ms: 12,
        });
        assert_eq!(app.cmd_log.len(), 1);
        assert!(!app.cmd_log[0].ok);
        assert_eq!(app.cmd_log[0].ms, 12);
        // The reason lives in the log, not on the bar.
        assert_eq!(app.cmd_log[0].output, ["error: the branch is not merged"]);
        assert!(app.message.is_empty(), "no command may hide the key hints");
        app.apply_resp(Resp::WriteDone {
            ok: true,
            cmd: "git add -A".into(),
            output: Vec::new(),
            ms: 3,
        });
        assert!(app.message.is_empty());
        assert!(app.cmd_log[1].output.is_empty());
    }

    #[test]
    fn hunk_view_marks_lines() {
        let mut app = demo();
        app.mode = Mode::Hunks {
            path: "src/app.rs".into(),
            header: "diff --git a/src/app.rs b/src/app.rs\n".into(),
            hunks: vec!["@@ -1,2 +1,3 @@\n keep\n-old line\n+new line\n".into()],
            cursor: 0,
            line: 1,
            picked: vec![1],
        };
        insta::assert_snapshot!(draw(&app, 100, 24).backend());
    }

    #[test]
    fn worktree_list() {
        let mut app = demo();
        app.mode = Mode::Worktrees {
            list: vec![
                WorktreeEntry { path: "/home/max/lazier".into(), branch: "main".into(), current: true },
                WorktreeEntry { path: "/home/max/lazier-fix".into(), branch: "fix/x".into(), current: false },
            ],
            cursor: 1,
        };
        insta::assert_snapshot!(draw(&app, 100, 24).backend());
    }

    // Before a bisect runs, skip and reset must do nothing. The bad key
    // starts the bisect. During a bisect the keys need no commit id.
    #[test]
    fn bisect_keys_need_a_running_bisect() {
        let id = Some("abc1234");
        assert_eq!(bisect_command(false, &Action::BisectSkip, id), None);
        assert_eq!(bisect_command(false, &Action::BisectReset, id), None);
        assert_eq!(
            bisect_command(false, &Action::BisectBad, id),
            Some(svec(&["bisect", "start", "abc1234"]))
        );
        assert_eq!(
            bisect_command(true, &Action::BisectBad, None),
            Some(svec(&["bisect", "bad"]))
        );
        assert_eq!(
            bisect_command(true, &Action::BisectReset, None),
            Some(svec(&["bisect", "reset"]))
        );
        // With no commit in the list, a bisect cannot start.
        assert_eq!(bisect_command(false, &Action::BisectBad, None), None);
    }

    // A click moves the focus to the panel under the pointer, and puts the
    // selection on the row that was clicked.
    #[test]
    fn a_click_moves_the_focus_and_the_selection() {
        use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let mut app = demo();
        app.area = ratatui::layout::Rect::new(0, 0, 80, 30);
        let p = ui::panes(app.area, app.show_log);
        let click = |x, y| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        };
        // The branches panel is the third box on the left.
        let b = p.left[2];
        app.handle_mouse(click(b.x + 4, b.y + 1));
        assert_eq!(app.focus, 2, "the click must move the focus");
        assert_eq!(app.selected[2], 0, "the first row is under that point");
        app.handle_mouse(click(b.x + 4, b.y + 3));
        assert_eq!(app.selected[2], 2, "the third row is under that point");

        // A click in the diff pane moves the focus there.
        app.handle_mouse(click(p.diff.x + 2, p.diff.y + 2));
        assert_eq!(app.focus, 5);
    }

    #[test]
    fn the_wheel_moves_the_selection() {
        use ratatui::crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
        let mut app = demo();
        app.area = ratatui::layout::Rect::new(0, 0, 80, 30);
        let p = ui::panes(app.area, app.show_log);
        let wheel = |kind, y| MouseEvent { kind, column: p.left[2].x + 2, row: y, modifiers: KeyModifiers::NONE };
        app.handle_mouse(wheel(MouseEventKind::ScrollDown, p.left[2].y + 1));
        assert_eq!((app.focus, app.selected[2]), (2, 1));
        app.handle_mouse(wheel(MouseEventKind::ScrollUp, p.left[2].y + 1));
        assert_eq!(app.selected[2], 0);
    }

    // The spinner turns only while a command runs, and it repeats.
    #[test]
    fn the_spinner_turns_only_when_busy() {
        let mut app = demo();
        assert_eq!(app.spinner(), None, "nothing runs, thus no spinner");
        app.apply(Action::Push);
        let first = app.spinner().expect("a push must show a spinner");
        app.tick += 1;
        assert_ne!(app.spinner(), Some(first), "the next tick shows another frame");
        app.tick += 9;
        assert_eq!(app.spinner(), Some(first), "ten frames make a full turn");
        app.apply_resp(Resp::WriteDone {
            ok: true,
            cmd: "git push".into(),
            output: Vec::new(),
            ms: 12,
        });
        assert_eq!(app.spinner(), None, "the spinner stops with the command");
    }

    #[test]
    fn the_branch_row_shows_the_spinner() {
        let mut app = demo();
        app.focus = 2;
        app.apply(Action::Push);
        insta::assert_snapshot!(draw(&app, 80, 24).backend());
    }

    // A network command shows in the bar while it runs, then goes away
    // when the result arrives.
    #[test]
    fn a_running_command_shows_and_then_clears() {
        let mut app = demo();
        app.apply(Action::Push);
        assert_eq!(app.running, ["git push"]);
        app.apply_resp(Resp::WriteDone {
            ok: true,
            cmd: "git push".into(),
            output: vec!["To github.com:max/lazier.git".into()],
            ms: 900,
        });
        assert!(app.running.is_empty());
        assert_eq!(app.cmd_log[0].output.len(), 1);
    }

    #[test]
    fn a_busy_branch_offers_its_worktree() {
        let mut app = demo();
        app.apply_resp(Resp::WriteDone {
            ok: false,
            cmd: "git checkout max/pratt".into(),
            output: vec![
                "fatal: 'max/pratt' is already used by worktree at '/home/max/pratt'".into(),
            ],
            ms: 31,
        });
        match &app.mode {
            Mode::Confirm { action: ConfirmAction::GoToWorktree(p), .. } => {
                assert_eq!(p, "/home/max/pratt")
            }
            _ => panic!("expected the offer to go to the worktree"),
        }
    }

    // A long branch name must not push the counts off the panel.
    #[test]
    fn long_branch_names_keep_their_counts() {
        let mut app = demo();
        app.focus = 2;
        app.repo.branches = [
            ("max/macro-warehouse-selection-and-more", true, 5u32, 0u32),
            ("max/pratt-parser-cleanup-with-a-very-long-tail", false, 12, 3),
            ("main", false, 0, 0),
        ]
        .into_iter()
        .map(|(name, current, ahead, behind)| BranchEntry {
            name: name.into(),
            current,
            ahead,
            behind,
            gone: false,
            age: "2d".into(),
        })
        .collect();
        insta::assert_snapshot!(draw(&app, 80, 24).backend());
    }

    #[test]
    fn confirm_window() {
        let mut app = demo();
        app.mode = Mode::Confirm {
            prompt: "that branch is checked out at /Users/max/dbt/fs-warehouses. go there?"
                .into(),
            action: ConfirmAction::GoToWorktree("/Users/max/dbt/fs-warehouses".into()),
        };
        insta::assert_snapshot!(draw(&app, 100, 30).backend());
    }

    #[test]
    fn other_errors_open_no_window() {
        let mut app = demo();
        app.apply_resp(Resp::WriteDone {
            ok: false,
            cmd: "git checkout nope".into(),
            output: vec!["error: pathspec 'nope' did not match".into()],
            ms: 5,
        });
        assert!(matches!(app.mode, Mode::Normal));
    }

    // A checkout of the branch you are on reads every file for nothing.
    #[test]
    fn no_checkout_of_the_branch_you_are_on() {
        let mut app = demo();
        app.focus = 2;
        app.selected[2] = 0;
        assert!(app.repo.branches[0].current);
        app.apply(Action::Checkout);
        assert!(app.running.is_empty());
        assert!(app.message.contains("you are already on"), "{}", app.message);
        assert!(app.message_ok, "this is not an error");
    }

    #[test]
    fn the_current_branch_is_first() {
        let app = demo();
        assert!(app.repo.branches[0].current);
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
    fn branch_prompt() {
        let mut app = demo();
        app.focus = 2;
        app.mode = Mode::Input {
            prompt: "new branch name",
            buffer: "feature/x".into(),
            purpose: InputPurpose::NewBranch,
        };
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
