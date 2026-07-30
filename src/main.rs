mod app;
mod event;
mod keys;
mod ui;

fn main() -> anyhow::Result<()> {
    let mut app = app::App::new();
    ratatui::run(|terminal| app.run(terminal))
}
