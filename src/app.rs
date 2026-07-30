use anyhow::Result;
use ratatui::crossterm::event::KeyEventKind;
use ratatui::DefaultTerminal;
use std::sync::mpsc;

use crate::event::{self, Msg};
use crate::keys::{action_for, Action};
use crate::ui;

pub const PANELS: [&str; 5] = ["Status", "Files", "Branches", "Commits", "Stash"];

pub struct App {
    pub focus: usize,
    pub selected: [usize; 5],
    pub quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self { focus: 1, selected: [0; 5], quit: false }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        event::spawn_input(tx);
        while !self.quit {
            terminal.draw(|f| ui::render(f, self))?;
            self.update(rx.recv()?);
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
            _ => {} // Resize: the next draw pass re-renders at the new size
        }
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Quit => self.quit = true,
            Action::NextPanel => self.focus = (self.focus + 1) % PANELS.len(),
            Action::PrevPanel => self.focus = (self.focus + PANELS.len() - 1) % PANELS.len(),
            Action::FocusPanel(i) => self.focus = i,
            Action::Down => {
                let sel = &mut self.selected[self.focus];
                if *sel + 1 < ui::panel_len(self.focus) {
                    *sel += 1;
                }
            }
            Action::Up => {
                let sel = &mut self.selected[self.focus];
                *sel = sel.saturating_sub(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn draw(app: &App, width: u16, height: u16) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| ui::render(f, app)).unwrap();
        terminal
    }

    #[test]
    fn layout_80x24() {
        insta::assert_snapshot!(draw(&App::new(), 80, 24).backend());
    }

    #[test]
    fn layout_200x50() {
        insta::assert_snapshot!(draw(&App::new(), 200, 50).backend());
    }

    #[test]
    fn commits_panel_scrolled_deep() {
        // selection far past one screen — proves the list virtualizes
        let mut app = App::new();
        app.focus = 3;
        app.selected[3] = 99_999;
        insta::assert_snapshot!(draw(&app, 80, 24).backend());
    }

    #[test]
    fn navigation() {
        let mut app = App::new();
        app.apply(Action::Down);
        assert_eq!(app.selected[1], 1);
        app.apply(Action::Up);
        app.apply(Action::Up); // clamps at 0
        assert_eq!(app.selected[1], 0);
        app.apply(Action::FocusPanel(3));
        assert_eq!(app.focus, 3);
        app.apply(Action::NextPanel);
        app.apply(Action::NextPanel); // wraps 4 -> 0
        assert_eq!(app.focus, 0);
        app.apply(Action::PrevPanel);
        assert_eq!(app.focus, 4);
    }
}
