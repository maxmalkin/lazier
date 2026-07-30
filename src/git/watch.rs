//! Watch the .git directory. Send a refresh message when HEAD, the index,
//! or a reference changes.
// ponytail: watch .git only, not the worktree. A recursive worktree watch
// makes too many file handles on a large repository. The worktree state
// refreshes after each command and on the r key. Add fsmonitor if that
// feels stale.
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

use crate::event::Msg;

fn interesting(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name == "HEAD" || name == "index" || path.components().any(|c| c.as_os_str() == "refs")
}

pub fn spawn(git_dir: PathBuf, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let (wtx, wrx) = std::sync::mpsc::channel();
        let Ok(mut watcher) = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res
                && ev.paths.iter().any(|p| interesting(p))
            {
                let _ = wtx.send(());
            }
        }) else {
            return;
        };
        if watcher.watch(&git_dir, RecursiveMode::Recursive).is_err() {
            return;
        }
        // Collect event bursts. Send one refresh for each burst.
        while wrx.recv().is_ok() {
            while wrx.recv_timeout(Duration::from_millis(200)).is_ok() {}
            if tx.send(Msg::Refresh).is_err() {
                break;
            }
        }
    });
}
