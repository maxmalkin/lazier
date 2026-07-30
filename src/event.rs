//! This is the one message channel for the application. The main loop does a
//! blocking receive on it. Thus the process uses no CPU when it is idle. The
//! input thread and the git workers send to clones of the same sender.
use ratatui::crossterm::event::{self, Event, KeyEvent};
use std::sync::mpsc::Sender;

use crate::git::Resp;

pub enum Msg {
    Key(KeyEvent),
    Resize,
    Git(Resp),
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
