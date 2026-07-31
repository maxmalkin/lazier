//! This is the git backend. Reads use gix. Only read.rs can import gix.
//! Writes and display diffs use the git subprocess.
pub mod graph;
pub mod patch;
mod read;
pub mod rebase;
pub mod watch;

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

use crate::event::Msg;

pub struct FileEntry {
    pub mark: char,
    pub staged: bool,
    pub path: String,
}

pub struct BranchEntry {
    pub name: String,
    pub current: bool,
    pub ahead: u32,
    pub behind: u32,
    /// The upstream branch is not there any more.
    pub gone: bool,
    /// The age of the last commit, in a short form such as "3d".
    pub age: String,
}

// Keep this struct small. The list can hold more than one million entries.
// The short id is inline. Box<str> fields save the capacity word of String.
pub struct CommitEntry {
    pub id: [u8; 7],
    pub graph: Box<str>,
    pub subject: Box<str>,
    pub author: Box<str>,
    pub time: u32,
}

impl CommitEntry {
    pub fn id_str(&self) -> &str {
        // The id always contains hex digits, thus this cannot fail.
        std::str::from_utf8(&self.id).unwrap_or("???????")
    }
}

pub enum LogReq {
    Chunk(usize),
    Reset,
}

pub enum Req {
    Status,
    Branches,
    Stashes,
    LogChunk { count: usize },
    LogReset,
    Diff { seq: u64, target: DiffTarget },
    /// Run a git command with the given arguments. Capture the output.
    Write(Vec<String>),
    /// Apply a patch to the index. Reverse removes it from the index.
    ApplyPatch { patch: String, reverse: bool },
    /// Read the sync state against the upstream branch.
    Sync,
    /// Read the full message of a commit, for the reword window.
    ReadMessage { id: String, index: usize },
    /// List the worktrees of the repository.
    Worktrees,
}

pub struct WorktreeEntry {
    pub path: String,
    pub branch: String,
    pub current: bool,
}

#[derive(PartialEq, Clone)]
pub enum DiffTarget {
    WorktreeFile(String),
    Commit(String),
}

pub enum Resp {
    Status(Vec<FileEntry>),
    Branches { current: Option<String>, entries: Vec<BranchEntry> },
    Stashes(Vec<String>),
    LogChunk { entries: Vec<CommitEntry>, done: bool },
    Diff { seq: u64, text: String },
    WriteDone { ok: bool, cmd: String, msg: String, ms: u64 },
    Sync { ahead: u32, behind: u32, unpushed: std::collections::HashSet<String> },
    Message { text: String, index: usize },
    Worktrees(Vec<WorktreeEntry>),
}

pub struct Git {
    tx: Sender<Req>,
    log_tx: Sender<LogReq>,
    pub root: PathBuf,
    pub git_dir: PathBuf,
}

impl Git {
    pub fn send(&self, req: Req) {
        match req {
            Req::LogChunk { count } => {
                let _ = self.log_tx.send(LogReq::Chunk(count));
            }
            Req::LogReset => {
                let _ = self.log_tx.send(LogReq::Reset);
            }
            _ => {
                let _ = self.tx.send(req);
            }
        }
    }
}

/// Find the repository at the current directory. Start two read workers.
/// One worker owns the log walker, which must stay alive between requests.
/// The other worker does all other reads and all writes.
pub fn spawn(event_tx: Sender<Msg>) -> anyhow::Result<Git> {
    let shared = Arc::new(gix::ThreadSafeRepository::discover(".")?);
    let root: PathBuf = shared
        .work_dir()
        .map(Into::into)
        .unwrap_or_else(|| shared.path().into());
    let git_dir: PathBuf = shared.path().into();

    let (tx, rx) = channel::<Req>();
    let (log_tx, log_rx) = channel::<LogReq>();

    let (repo, ev) = (shared.clone(), event_tx.clone());
    std::thread::spawn(move || read::log_thread(repo, log_rx, ev));

    let worker_root = root.clone();
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
                Req::Branches => Some(Resp::Branches {
                    current: read::head_name(&repo),
                    entries: branches(&worker_root),
                }),
                Req::Stashes => read::stashes(&repo),
                Req::Diff { seq, target } => Some(Resp::Diff { seq, text: display_diff(&worker_root, &target) }),
                Req::Write(args) => Some(run_git(&worker_root, &args)),
                Req::ApplyPatch { patch, reverse } => Some(apply_patch(&worker_root, &patch, reverse)),
                Req::Sync => Some(sync_state(&worker_root)),
                Req::Worktrees => Some(Resp::Worktrees(worktrees(&worker_root))),
                Req::ReadMessage { id, index } => {
                    let out = Command::new("git")
                        .arg("-C")
                        .arg(&worker_root)
                        .args(["log", "-1", "--format=%B", &id])
                        .output();
                    let text = out
                        .ok()
                        .filter(|o| o.status.success())
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim_end().to_string())
                        .unwrap_or_default();
                    Some(Resp::Message { text, index })
                }
                Req::LogChunk { .. } | Req::LogReset => None,
            };
            if let Some(resp) = resp
                && event_tx.send(Msg::Git(resp)).is_err()
            {
                break;
            }
        }
    });

    Ok(Git { tx, log_tx, root, git_dir })
}

fn run_git(root: &PathBuf, args: &[String]) -> Resp {
    let cmd = format!("git {}", args.join(" "));
    let start = std::time::Instant::now();
    let result = Command::new("git").arg("-C").arg(root).args(args).output();
    let ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(out) => {
            let ok = out.status.success();
            let text = if ok { &out.stdout } else { &out.stderr };
            // The log shows the command on its own row, thus the message
            // needs no command name in front of it.
            let msg = String::from_utf8_lossy(text).lines().next().unwrap_or("done").to_string();
            Resp::WriteDone { ok, cmd, msg, ms }
        }
        Err(e) => Resp::WriteDone { ok: false, cmd, msg: e.to_string(), ms },
    }
}

fn apply_patch(root: &PathBuf, patch: &str, reverse: bool) -> Resp {
    let start = std::time::Instant::now();
    let mut args = vec!["apply", "--cached"];
    if reverse {
        args.push("-R");
    }
    let cmd = format!("git {}", args.join(" "));
    let child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(&args)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let out = match child {
        Ok(mut child) => {
            let _ = child.stdin.take().unwrap().write_all(patch.as_bytes());
            child.wait_with_output().map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    };
    let ms = start.elapsed().as_millis() as u64;
    match out {
        Ok(out) if out.status.success() => {
            let verb = if reverse { "unstaged" } else { "staged" };
            Resp::WriteDone { ok: true, cmd, msg: format!("hunk {verb}"), ms }
        }
        Ok(out) => Resp::WriteDone {
            ok: false,
            cmd,
            msg: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ms,
        },
        Err(e) => Resp::WriteDone { ok: false, cmd, msg: e, ms },
    }
}

/// List the local branches, the newest commit first. One call gives the
/// order, the current branch, and the distance to each upstream branch.
fn branches(root: &PathBuf) -> Vec<BranchEntry> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:short)\t%(HEAD)\t%(upstream:track)\t%(committerdate:relative)",
            "refs/heads/",
        ])
        .output();
    let Ok(out) = out else { return Vec::new() };
    let mut list: Vec<BranchEntry> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let name = parts.next()?.to_string();
            let current = parts.next() == Some("*");
            // The track field looks like "[ahead 2, behind 1]" or "[gone]".
            let track = parts.next().unwrap_or("");
            Some(BranchEntry {
                name,
                current,
                ahead: track_count(track, "ahead "),
                behind: track_count(track, "behind "),
                gone: track.contains("gone"),
                age: short_age(parts.next().unwrap_or("")),
            })
        })
        .collect();
    // The branch you are on goes first. The others keep the recency order.
    if let Some(i) = list.iter().position(|b| b.current) {
        let current = list.remove(i);
        list.insert(0, current);
    }
    list
}

/// List the worktrees. The output has one block for each worktree. A block
/// has a "worktree" line and often a "branch" line.
fn worktrees(root: &PathBuf) -> Vec<WorktreeEntry> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["worktree", "list", "--porcelain"])
        .output();
    let Ok(out) = out else { return Vec::new() };
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    // Compare real paths. A symbolic link makes two names for one directory.
    let here = root.canonicalize().unwrap_or_else(|_| root.clone());
    let mut list = Vec::new();
    for block in text.split("\n\n") {
        let mut path = String::new();
        let mut branch = String::from("(detached)");
        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = p.to_string();
            } else if let Some(b) = line.strip_prefix("branch ") {
                branch = b.trim_start_matches("refs/heads/").to_string();
            }
        }
        if !path.is_empty() {
            let real = PathBuf::from(&path);
            let current = real.canonicalize().unwrap_or(real) == here;
            list.push(WorktreeEntry { path, branch, current });
        }
    }
    list
}

/// Make a short age from the words that git gives. "3 days ago" gives "3d".
fn short_age(text: &str) -> String {
    let mut words = text.split_whitespace();
    let Some(n) = words.next() else { return String::new() };
    let Some(unit) = words.next() else { return String::new() };
    // "2 years, 3 months ago" keeps only the first part.
    let letter = unit.trim_end_matches(',').chars().next().unwrap_or(' ');
    match n.parse::<u32>() {
        Ok(n) => format!("{n}{letter}"),
        // Git also says "just now" and similar.
        Err(_) => "now".into(),
    }
}

fn track_count(track: &str, word: &str) -> u32 {
    match track.split_once(word) {
        Some((_, rest)) => rest.trim_start().split(|c: char| !c.is_ascii_digit()).next().unwrap_or("").parse().unwrap_or(0),
        None => 0,
    }
}

// Read ahead/behind counts and the set of commits that are not on the
// upstream branch. No upstream gives zero counts and an empty set.
fn sync_state(root: &PathBuf) -> Resp {
    let run = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let (mut behind, mut ahead) = (0, 0);
    if let Some(out) = run(&["rev-list", "--left-right", "--count", "@{upstream}...HEAD"])
        && let Some((b, a)) = out.trim().split_once('\t')
    {
        behind = b.parse().unwrap_or(0);
        ahead = a.parse().unwrap_or(0);
    }
    let unpushed = run(&["rev-list", "--abbrev-commit", "--abbrev=7", "@{upstream}..HEAD"])
        .map(|out| out.lines().map(|l| l.chars().take(7).collect()).collect())
        .unwrap_or_default();
    Resp::Sync { ahead, behind, unpushed }
}

fn display_diff(root: &PathBuf, target: &DiffTarget) -> String {
    // The worktree diff is index-to-worktree. The hunk staging mode applies
    // these hunks with `apply --cached`, thus the base must be the index.
    let args: Vec<&str> = match target {
        DiffTarget::WorktreeFile(p) => vec!["diff", "--", p],
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
