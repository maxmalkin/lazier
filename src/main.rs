mod app;
mod event;
mod git;
mod keys;
mod ui;


fn main() -> anyhow::Result<()> {
    // Accept a repository path as the first argument. The benchmark harness
    // uses this.
    if let Some(dir) = std::env::args().nth(1) {
        std::env::set_current_dir(dir)?;
    }
    let mut app = app::App::new();
    ratatui::run(|terminal| app.run(terminal))
}
