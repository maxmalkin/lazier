mod app;
mod event;
mod git;
mod keys;
mod tree;
mod ui;


fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    // Git calls this program as the sequence editor of an interactive
    // rebase. Copy the prepared todo list over the file that git made, then
    // exit. No terminal user interface starts.
    if args.get(1).map(String::as_str) == Some("--seq-editor") {
        std::fs::copy(&args[2], &args[3])?;
        return Ok(());
    }
    // Accept a repository path as the first argument. The benchmark harness
    // uses this.
    if let Some(dir) = args.get(1) {
        std::env::set_current_dir(dir)?;
    }
    let mut app = app::App::new();
    ratatui::run(|terminal| app.run(terminal))
}
