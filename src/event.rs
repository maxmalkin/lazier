//! This is the one message channel for the application. The main loop does a
//! blocking receive on it. The input thread and the git workers send to
//! clones of the same sender.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyEvent, MouseEvent};

use crate::git::Resp;

pub enum Msg {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize,
    Git(Resp),
    Refresh,
}

/// Start the input thread. When `pause` is set, the thread does not read
/// the terminal. The main loop sets it before it runs a child process that
/// needs the terminal, for example git push.
// Poll instead of a blocking read. A blocking read would steal terminal
// input from child processes, for example a git credential prompt.
pub fn spawn_input(tx: Sender<Msg>, pause: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        loop {
            if pause.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            if !event::poll(Duration::from_millis(200)).unwrap_or(false) {
                continue;
            }
            let msg = match event::read() {
                Ok(Event::Key(k)) => Msg::Key(k),
                Ok(Event::Mouse(m)) => Msg::Mouse(m),
                Ok(Event::Resize(..)) => Msg::Resize,
                Ok(_) => continue,
                Err(_) => break,
            };
            if tx.send(msg).is_err() {
                break;
            }
        }
    });
}
