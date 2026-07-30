use ratatui::crossterm::event::{KeyCode, KeyEvent};

pub enum Action {
    Quit,
    NextPanel,
    PrevPanel,
    FocusPanel(usize),
    Up,
    Down,
    Refresh,
}

pub fn action_for(key: KeyEvent) -> Option<Action> {
    Some(match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
        KeyCode::Tab => Action::NextPanel,
        KeyCode::BackTab => Action::PrevPanel,
        KeyCode::Char(c @ '1'..='5') => Action::FocusPanel(c as usize - '1' as usize),
        KeyCode::Char('j') | KeyCode::Down => Action::Down,
        KeyCode::Char('k') | KeyCode::Up => Action::Up,
        KeyCode::Char('r') => Action::Refresh,
        _ => return None,
    })
}
