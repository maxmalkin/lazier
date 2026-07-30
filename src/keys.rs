use ratatui::crossterm::event::{KeyCode, KeyEvent};

pub enum Action {
    Quit,
    NextPanel,
    PrevPanel,
    FocusPanel(usize),
    Up,
    Down,
    Refresh,
    // Files panel
    ToggleStage,
    StageAll,
    CommitPrompt,
    CommitEditor,
    StashPrompt,
    EnterHunks,
    TakeOurs,
    TakeTheirs,
    // Branches panel
    Checkout,
    NewBranchPrompt,
    DeleteBranch,
    Push,
    Pull,
    Fetch,
    // Stash panel
    ApplyStash,
    PopStash,
    DropStash,
}

/// Map a key to an action. Global keys come first. Panel keys depend on the
/// panel in focus.
pub fn action_for(key: KeyEvent, focus: usize) -> Option<Action> {
    let global = match key.code {
        KeyCode::Char('q') => Some(Action::Quit),
        KeyCode::Tab => Some(Action::NextPanel),
        KeyCode::BackTab => Some(Action::PrevPanel),
        KeyCode::Char(c @ '1'..='5') => Some(Action::FocusPanel(c as usize - '1' as usize)),
        KeyCode::Char('j') | KeyCode::Down => Some(Action::Down),
        KeyCode::Char('k') | KeyCode::Up => Some(Action::Up),
        KeyCode::Char('r') => Some(Action::Refresh),
        _ => None,
    };
    if global.is_some() {
        return global;
    }
    match (focus, key.code) {
        (1, KeyCode::Char(' ')) => Some(Action::ToggleStage),
        (1, KeyCode::Char('a')) => Some(Action::StageAll),
        (1, KeyCode::Char('c')) => Some(Action::CommitPrompt),
        (1, KeyCode::Char('C')) => Some(Action::CommitEditor),
        (1, KeyCode::Char('s')) => Some(Action::StashPrompt),
        (1, KeyCode::Enter) => Some(Action::EnterHunks),
        (1, KeyCode::Char('o')) => Some(Action::TakeOurs),
        (1, KeyCode::Char('t')) => Some(Action::TakeTheirs),
        (2, KeyCode::Enter) => Some(Action::Checkout),
        (2, KeyCode::Char('n')) => Some(Action::NewBranchPrompt),
        (2, KeyCode::Char('d')) => Some(Action::DeleteBranch),
        (2, KeyCode::Char('P')) => Some(Action::Push),
        (2, KeyCode::Char('p')) => Some(Action::Pull),
        (2, KeyCode::Char('f')) => Some(Action::Fetch),
        (4, KeyCode::Enter | KeyCode::Char('a')) => Some(Action::ApplyStash),
        (4, KeyCode::Char('p')) => Some(Action::PopStash),
        (4, KeyCode::Char('d')) => Some(Action::DropStash),
        _ => None,
    }
}
