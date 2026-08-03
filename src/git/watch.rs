//! Watch the .git directory. Send a refresh message when HEAD, the index,
//! or a reference changes.
// Watch .git only, not the worktree. A recursive worktree watch makes too
// many file handles on a large repository. The worktree state refreshes
// after each command and on the r key.
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

/// The most paths to carry in one message. Past this it is cheaper to look
/// at the whole work tree than to name every path.
const MAX_PATHS: usize = 300;

/// Watch the work tree and name the files that changed. The status scan can
/// then look at those files only, which is far less work than a walk of a
/// large work tree.
///
/// The watch may fail, and events may be lost when many files change at
/// once. Both cases send `None`, which asks for a full scan.
pub fn spawn_worktree(root: PathBuf, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let (wtx, wrx) = std::sync::mpsc::channel::<Option<PathBuf>>();
        let Ok(mut watcher) = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            match res {
                Ok(ev) => {
                    // The watcher lost events, thus what changed is unknown.
                    if matches!(ev.kind, notify::EventKind::Other) {
                        let _ = wtx.send(None);
                        return;
                    }
                    for p in ev.paths {
                        let _ = wtx.send(Some(p));
                    }
                }
                // An error leaves the truth unknown, thus ask for a full scan.
                Err(_) => {
                    let _ = wtx.send(None);
                }
            }
        }) else {
            return;
        };
        // A work tree that cannot be watched keeps the old behaviour: every
        // refresh looks at every file.
        if watcher.watch(&root, RecursiveMode::Recursive).is_err() {
            return;
        }
        while let Ok(first) = wrx.recv() {
            let mut paths = Vec::new();
            let mut unknown = first.is_none();
            if let Some(p) = first {
                paths.push(p);
            }
            // Collect the burst that follows one save in an editor.
            while let Ok(next) = wrx.recv_timeout(Duration::from_millis(150)) {
                match next {
                    Some(p) => paths.push(p),
                    None => unknown = true,
                }
            }
            let names = (!unknown)
                .then(|| relative_names(&root, paths))
                .flatten();
            if tx.send(Msg::Dirty(names)).is_err() {
                return;
            }
        }
    });
}

/// Turn the paths into names under the root. None means the set is not
/// usable, thus the caller must look at every file.
fn relative_names(root: &Path, paths: Vec<PathBuf>) -> Option<Vec<String>> {
    let real_root = root.canonicalize().ok();
    let mut out: Vec<String> = Vec::new();
    for p in paths {
        // Work inside .git is not work-tree content.
        if p.components().any(|c| c.as_os_str() == ".git") {
            continue;
        }
        let rel = p
            .strip_prefix(root)
            .ok()
            .or_else(|| real_root.as_deref().and_then(|r| p.strip_prefix(r).ok()))?;
        let name = rel.to_str()?;
        if name.is_empty() {
            continue;
        }
        out.push(name.to_string());
        if out.len() > MAX_PATHS {
            return None;
        }
    }
    (!out.is_empty()).then_some(out)
}
