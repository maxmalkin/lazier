//! This is the git backend. Reads use gix. Only read.rs can import gix.
//! Display diffs use the git subprocess.
mod read;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

use crate::event::Msg;

pub struct FileEntry {
    pub mark: char,
    pub staged: bool,
    pub path: String,
}

// Keep this struct small. The list can hold more than one million entries.
// The short id is inline, thus each entry makes only one heap allocation.
pub struct CommitEntry {
    pub id: [u8; 7],
    pub subject: Box<str>,
}

impl CommitEntry {
    pub fn id_str(&self) -> &str {
        // The id always contains hex digits, thus this cannot fail.
        std::str::from_utf8(&self.id).unwrap_or("???????")
    }
}

pub enum Req {
    Status,
    Branches,
    Stashes,
    LogChunk { count: usize },
    Diff { seq: u64, target: DiffTarget },
}

#[derive(PartialEq, Clone)]
pub enum DiffTarget {
    WorktreeFile(String),
    Commit(String),
}

pub enum Resp {
    Status(Vec<FileEntry>),
    Branches { current: Option<String>, names: Vec<String> },
    Stashes(Vec<String>),
    LogChunk { entries: Vec<CommitEntry>, done: bool },
    Diff { seq: u64, text: String },
}

pub struct Git {
    tx: Sender<Req>,
    log_tx: Sender<usize>,
}

impl Git {
    pub fn send(&self, req: Req) {
        match req {
            Req::LogChunk { count } => {
                let _ = self.log_tx.send(count);
            }
            _ => {
                let _ = self.tx.send(req);
            }
        }
    }
}

/// Find the repository at the current directory. Start two read workers.
/// One worker owns the log walker, which must stay alive between requests.
/// The other worker does all other reads.
// ponytail: two threads only. Add a thread pool if the profile shows the need.
pub fn spawn(event_tx: Sender<Msg>) -> anyhow::Result<Git> {
    let shared = Arc::new(gix::ThreadSafeRepository::discover(".")?);
    let root: PathBuf = shared
        .work_dir()
        .map(Into::into)
        .unwrap_or_else(|| shared.path().into());

    let (tx, rx) = channel::<Req>();
    let (log_tx, log_rx) = channel::<usize>();

    let (repo, ev) = (shared.clone(), event_tx.clone());
    std::thread::spawn(move || read::log_thread(repo, log_rx, ev));

    std::thread::spawn(move || {
        let repo = shared.to_thread_local();
        for req in rx {
            let resp = match req {
                // A status scan can be slow on a large repository. Run it in
                // its own thread. Then the other reads do not wait for it.
                Req::Status => {
                    let (sh, ev) = (shared.clone(), event_tx.clone());
                    std::thread::spawn(move || {
                        let repo = sh.to_thread_local();
                        if let Some(resp) = read::status(&repo) {
                            let _ = ev.send(Msg::Git(resp));
                        }
                    });
                    None
                }
                Req::Branches => read::branches(&repo),
                Req::Stashes => read::stashes(&repo),
                Req::Diff { seq, target } => Some(Resp::Diff { seq, text: display_diff(&root, &target) }),
                Req::LogChunk { .. } => None,
            };
            if let Some(resp) = resp
                && event_tx.send(Msg::Git(resp)).is_err()
            {
                break;
            }
        }
    });

    Ok(Git { tx, log_tx })
}

// ponytail: the display diff uses a subprocess. This is not the hot path.
// Move it to gix blob diffing if the profile shows a cost.
fn display_diff(root: &PathBuf, target: &DiffTarget) -> String {
    let args: Vec<&str> = match target {
        DiffTarget::WorktreeFile(p) => vec!["diff", "HEAD", "--", p],
        DiffTarget::Commit(id) => vec!["show", "--stat", "--patch", id],
    };
    match Command::new("git").arg("-C").arg(root).args(&args).output() {
        Ok(out) if out.status.success() && !out.stdout.is_empty() => {
            String::from_utf8_lossy(&out.stdout).into_owned()
        }
        Ok(out) => String::from_utf8_lossy(&out.stderr).into_owned(),
        Err(e) => e.to_string(),
    }
}
