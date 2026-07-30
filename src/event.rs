//! One channel, one loop. Workers and the fs watcher get clones of the same
//! Sender in phase 2+; the main loop blocks on recv — 0% CPU when idle.
use ratatui::crossterm::event::{self, Event, KeyEvent};
use std::sync::mpsc::Sender;

pub enum Msg {
    Key(KeyEvent),
    Resize,
}

pub fn spawn_input(tx: Sender<Msg>) {
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            let msg = match ev {
                Event::Key(k) => Msg::Key(k),
                Event::Resize(..) => Msg::Resize,
                _ => continue,
            };
            if tx.send(msg).is_err() {
                break;
            }
        }
    });
}
