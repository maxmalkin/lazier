use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::KeyEventKind;
use std::sync::mpsc;

use crate::event::{self, Msg};
use crate::git::{self, CommitEntry, DiffTarget, FileEntry, Git, Req, Resp};
use crate::keys::{Action, action_for};
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
}

pub struct App {
    pub focus: usize,
    pub selected: [usize; 5],
    pub quit: bool,
    pub repo: RepoState,
    git: Option<Git>,
    log_inflight: bool,
    diff_seq: u64,
    diff_target: Option<DiffTarget>,
}

impl App {
    pub fn new() -> Self {
        Self {
            focus: 1,
            selected: [0; 5],
            quit: false,
            repo: RepoState::default(),
            git: None,
            log_inflight: false,
            diff_seq: 0,
            diff_target: None,
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        event::spawn_input(tx.clone());
        let git = git::spawn(tx)?;
        for req in [Req::Status, Req::Branches, Req::Stashes] {
            git.send(req);
        }
        git.send(Req::LogChunk { count: LOG_CHUNK });
        self.log_inflight = true;
        self.git = Some(git);

        while !self.quit {
            terminal.draw(|f| ui::render(f, self))?;
            // Drain the queue before each draw. This makes one draw for a
            // burst of messages, not one draw for each message.
            self.update(rx.recv()?);
            while let Ok(msg) = rx.try_recv() {
                self.update(msg);
            }
            self.flush_requests();
        }
        Ok(())
    }

    pub fn update(&mut self, msg: Msg) {
        match msg {
            Msg::Key(key) if key.kind == KeyEventKind::Press => {
                if let Some(action) = action_for(key) {
                    self.apply(action);
                }
            }
            Msg::Git(resp) => self.apply_resp(resp),
            // A resize needs no work. The next draw uses the new size.
            _ => {}
        }
    }

    pub fn panel_len(&self, panel: usize) -> usize {
        match panel {
            0 => 1,
            1 => self.repo.files.len(),
            2 => self.repo.branches.len(),
            3 => self.repo.commits.len(),
            4 => self.repo.stashes.len(),
            _ => 0,
        }
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Quit => self.quit = true,
            Action::NextPanel => self.focus = (self.focus + 1) % PANELS.len(),
            Action::PrevPanel => self.focus = (self.focus + PANELS.len() - 1) % PANELS.len(),
            Action::FocusPanel(i) => self.focus = i,
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
            Action::Refresh => {
                if let Some(git) = &self.git {
                    for req in [Req::Status, Req::Branches, Req::Stashes] {
                        git.send(req);
                    }
                }
            }
        }
    }

    fn apply_resp(&mut self, resp: Resp) {
        match resp {
            Resp::Status(files) => self.repo.files = files,
            Resp::Branches { current, names } => {
                self.repo.head = current;
                self.repo.branches = names;
            }
            Resp::Stashes(stashes) => self.repo.stashes = stashes,
            Resp::LogChunk { entries, done } => {
                self.repo.commits.extend(entries);
                self.repo.log_done = done;
                self.log_inflight = false;
            }
            // Ignore a diff for an old selection. Only the last request counts.
            Resp::Diff { seq, text } => {
                if seq == self.diff_seq {
                    self.repo.diff = text;
                }
            }
        }
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
            1 => self.repo.files.get(self.selected[1]).map(|f| DiffTarget::WorktreeFile(f.path.clone())),
            3 => self.repo.commits.get(self.selected[3]).map(|c| DiffTarget::Commit(c.id.clone())),
            _ => None,
        };
        if target.is_some() && target != self.diff_target {
            self.diff_seq += 1;
            self.diff_target = target.clone();
            git.send(Req::Diff { seq: self.diff_seq, target: target.unwrap() });
        }
    }
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
        app.repo.files = [('M', true, "src/main.rs"), ('A', false, "src/app.rs"), ('?', false, "notes.txt")]
            .into_iter()
            .map(|(mark, staged, path)| FileEntry { mark, staged, path: path.into() })
            .collect();
        app.repo.branches = vec!["feature/ui".into(), "main".into()];
        app.repo.stashes = vec!["stash@{0}: WIP on main".into()];
        app.repo.commits = (0..100_000)
            .map(|i| CommitEntry { id: format!("{:07x}", 0xa0c000 + i), subject: format!("fake: commit subject #{i}") })
            .collect();
        app.repo.diff = "diff --git a/src/main.rs b/src/main.rs\n+added line\n-removed line".into();
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
        app.apply(Action::NextPanel); // The focus goes from panel 4 to panel 0.
        assert_eq!(app.focus, 0);
        app.apply(Action::PrevPanel);
        assert_eq!(app.focus, 4);
    }
}
