//! This is the git backend. Reads use gix. Only read.rs can import gix.
//! Writes and display diffs use the git subprocess.
pub mod absorb;
pub mod graph;
pub mod patch;
mod read;
pub mod rebase;
#[cfg(test)]
mod tests;
pub mod watch;

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};

use crate::event::Msg;

/// One row for each path. `index` is the state against HEAD. `work` is the
/// state against the index. A space means no change on that side. This is
/// the same shape as the two columns of `git status --short`.
pub struct FileEntry {
    pub index: char,
    pub work: char,
    pub path: String,
}

impl FileEntry {
    pub fn staged(&self) -> bool {
        self.index != ' '
    }
    pub fn conflicted(&self) -> bool {
        self.work == 'U' || self.index == 'U'
    }
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
    /// The branch lives on a remote. A checkout makes a local branch that
    /// follows it.
    pub remote: bool,
}

pub struct BlameLine {
    pub id: String,
    pub author: String,
    pub date: String,
    pub text: String,
}

pub struct SubmoduleEntry {
    pub path: String,
    pub id: String,
    /// A dash means the submodule has no checkout. A plus means it sits at
    /// another commit than the one the repository records.
    pub state: char,
}

pub struct ReflogEntry {
    pub id: String,
    /// The position, such as HEAD@{2}.
    pub at: String,
    pub what: String,
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
    /// Look at HEAD. Walk again only when HEAD moved. A refresh after a
    /// stage or a fetch must not throw away a long list of commits.
    Refresh(usize),
    /// Show only the commits whose message holds this text. None gives the
    /// whole history again.
    Filter(Option<String>),
    /// Follow only the first parent of each commit, or every parent. The
    /// walk starts again, thus the list shows the new shape at once.
    FirstParent(bool),
}

pub enum Req {
    Status,
    /// Look at these paths only. The answer replaces what is known about
    /// them and leaves every other path as it is.
    StatusPaths(Vec<String>),
    Branches,
    Stashes,
    LogChunk {
        count: usize,
    },
    LogRefresh {
        count: usize,
    },
    LogFilter(Option<String>),
    /// Follow only the first parent of each commit, or every parent.
    LogFirstParent(bool),
    Diff {
        seq: u64,
        target: DiffTarget,
    },
    /// Run a git command with the given arguments. Capture the output.
    Write(Vec<String>),
    /// Apply a patch to the index. Reverse removes it from the index.
    ApplyPatch {
        patch: String,
        reverse: bool,
    },
    /// Read the sync state against the upstream branch.
    Sync,
    /// Read the full message of a commit, for the reword window.
    ReadMessage {
        id: String,
        index: usize,
    },
    /// List the worktrees of the repository.
    Worktrees,
    /// List the recent positions of HEAD.
    Reflog,
    /// Put text on the clipboard of the system.
    Copy(String),
    /// Read the tags, so the commit rows can show them.
    Tags,
    /// Read who last changed each line of a file.
    Blame(String),
    /// List the submodules of the repository.
    Submodules,
    /// Send each staged change back to the commit that last wrote those
    /// lines. Only the commits in the set may take a change.
    Absorb {
        own: std::collections::HashSet<String>,
    },
    /// Run a command line through the shell, in the root of the repository.
    Shell(String),
    /// Add a pattern to the ignore rules. `local` writes to the private
    /// file of the repository, thus no other person sees the rule.
    Ignore {
        pattern: String,
        local: bool,
    },
}

pub struct WorktreeEntry {
    pub path: String,
    pub branch: String,
    pub current: bool,
    /// The first worktree of a repository. Git does not let you remove it.
    pub main: bool,
    pub locked: bool,
    /// The directory is gone. A prune removes the record.
    pub prunable: bool,
}

#[derive(PartialEq, Clone)]
pub enum DiffTarget {
    /// A file in the work tree. Git does not track a new file, thus its
    /// diff needs another command.
    WorktreeFile {
        path: String,
        untracked: bool,
    },
    Commit(String),
    /// A stash entry, by its position in the list.
    Stash(usize),
    /// Everything between two commits.
    Range {
        from: String,
        to: String,
    },
}

pub enum Resp {
    Status(Vec<FileEntry>),
    /// The state of the paths that were looked at. A path that was looked
    /// at and is not in `files` has no change any more.
    StatusPaths {
        scanned: Vec<String>,
        files: Vec<FileEntry>,
    },
    Branches {
        current: Option<String>,
        entries: Vec<BranchEntry>,
    },
    Stashes(Vec<String>),
    LogChunk {
        entries: Vec<CommitEntry>,
        done: bool,
    },
    /// HEAD moved. These entries take the place of the whole list.
    LogReplace {
        entries: Vec<CommitEntry>,
        done: bool,
    },
    /// `text` is the work-tree diff. `staged` is the index diff. A commit
    /// puts everything in `text` and leaves `staged` empty.
    Diff {
        seq: u64,
        text: String,
        staged: String,
    },
    /// `output` holds the first lines that the command printed. The command
    /// log shows them.
    WriteDone {
        ok: bool,
        cmd: String,
        output: Vec<String>,
        ms: u64,
    },
    /// A command that the program started on its own. It only writes to
    /// the log, thus it never opens a window over your work.
    Background {
        ok: bool,
        cmd: String,
        output: Vec<String>,
        ms: u64,
    },
    Sync {
        ahead: u32,
        behind: u32,
        unpushed: std::collections::HashSet<String>,
        /// The upstream branch, such as "origin/main".
        upstream: Option<String>,
    },
    Message {
        text: String,
        index: usize,
    },
    Worktrees(Vec<WorktreeEntry>),
    Reflog(Vec<ReflogEntry>),
    /// A tag name for each commit that carries one.
    Tags(std::collections::HashMap<String, Vec<String>>),
    Blame {
        path: String,
        lines: Vec<BlameLine>,
    },
    Submodules(Vec<SubmoduleEntry>),
}

pub struct Git {
    tx: Sender<Req>,
    log_tx: Sender<LogReq>,
    pub root: PathBuf,
    pub git_dir: PathBuf,
}

/// Add one line to a file of ignore rules. The file gets a newline first
/// when it does not end with one, thus the rules never join together.
fn append_rule(path: &std::path::Path, pattern: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let old = std::fs::read_to_string(path).unwrap_or_default();
    // A rule that is already there needs no second copy.
    if old.lines().any(|l| l.trim() == pattern) {
        return Ok(());
    }
    let mut text = old;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(pattern);
    text.push('\n');
    std::fs::write(path, text)
}

impl Git {
    pub fn send(&self, req: Req) {
        match req {
            Req::LogChunk { count } => {
                let _ = self.log_tx.send(LogReq::Chunk(count));
            }
            Req::LogRefresh { count } => {
                let _ = self.log_tx.send(LogReq::Refresh(count));
            }
            Req::LogFirstParent(on) => {
                let _ = self.log_tx.send(LogReq::FirstParent(on));
            }
            Req::LogFilter(text) => {
                let _ = self.log_tx.send(LogReq::Filter(text));
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
    let root: PathBuf = shared.work_dir().map(Into::into).unwrap_or_else(|| shared.path().into());
    // Discovery can give a relative path such as ".". A full path is needed
    // to name the parent directory and to compare worktrees.
    let root = root.canonicalize().unwrap_or(root);
    let git_dir: PathBuf = shared.path().into();
    // Every worktree of a repository shares the common directory.
    let common: PathBuf = shared.to_thread_local().common_dir().to_owned();

    let (tx, rx) = channel::<Req>();
    let (log_tx, log_rx) = channel::<LogReq>();

    let (repo, ev, log_root) = (shared.clone(), event_tx.clone(), root.clone());
    std::thread::spawn(move || read::log_thread(repo, log_root, log_rx, ev));
    spawn_auto_fetch(root.clone(), event_tx.clone());

    let worker_root = root.clone();
    let scanning = Arc::new(AtomicBool::new(false));
    std::thread::spawn(move || {
        let repo = shared.to_thread_local();
        for req in rx {
            let resp = match req {
                // A status scan can be slow on a large repository. Run it in
                // its own thread. Then the other reads do not wait for it.
                // One scan at a time is enough: a scan that starts later
                // sees the same work tree.
                Req::Status => {
                    if !scanning.swap(true, Ordering::AcqRel) {
                        let (sh, ev, flag) = (shared.clone(), event_tx.clone(), scanning.clone());
                        std::thread::spawn(move || {
                            let repo = sh.to_thread_local();
                            let resp = read::status(&repo);
                            flag.store(false, Ordering::Release);
                            if let Some(resp) = resp {
                                let _ = ev.send(Msg::Git(resp));
                            }
                        });
                    }
                    None
                }
                // A scan of a few paths is quick, thus it runs here rather
                // than on a thread of its own.
                Req::StatusPaths(paths) => read::status_paths(&repo, &paths),
                Req::Branches => Some(Resp::Branches {
                    current: read::head_name(&repo),
                    entries: branches(&worker_root),
                }),
                Req::Stashes => read::stashes(&repo),
                Req::Diff { seq, target } => {
                    let (text, staged) = display_diff(&worker_root, &target);
                    Some(Resp::Diff { seq, text, staged })
                }
                // A network command can take seconds. Give it its own
                // thread, thus the other reads do not wait for it.
                Req::Write(args) if is_network(&args) => {
                    let (root, ev) = (worker_root.clone(), event_tx.clone());
                    std::thread::spawn(move || {
                        let _ = ev.send(Msg::Git(run_git(&root, &args)));
                    });
                    None
                }
                Req::Write(args) => Some(run_git(&worker_root, &args)),
                Req::Shell(line) => {
                    let (root, ev) = (worker_root.clone(), event_tx.clone());
                    std::thread::spawn(move || {
                        let _ = ev.send(Msg::Git(run_shell(&root, &line)));
                    });
                    None
                }
                Req::ApplyPatch { patch, reverse } => {
                    Some(apply_patch(&worker_root, &patch, reverse))
                }
                Req::Sync => Some(sync_state(&worker_root)),
                Req::Worktrees => Some(Resp::Worktrees(worktrees(&worker_root))),
                Req::Reflog => Some(Resp::Reflog(reflog(&worker_root))),
                Req::Tags => read::tags(&repo),
                Req::Blame(path) => Some(Resp::Blame { lines: blame(&worker_root, &path), path }),
                Req::Submodules => Some(Resp::Submodules(submodules(&worker_root))),
                // The work can take seconds on a big change, thus it runs
                // on a thread of its own.
                Req::Absorb { own } => {
                    let (root, ev) = (worker_root.clone(), event_tx.clone());
                    std::thread::spawn(move || {
                        let start = std::time::Instant::now();
                        let result = absorb::run(&root, &|id| own.contains(&id[..7.min(id.len())]));
                        let ms = start.elapsed().as_millis() as u64;
                        let resp = match result {
                            Ok(lines) => Resp::WriteDone {
                                ok: true,
                                cmd: "absorb".into(),
                                output: lines,
                                ms,
                            },
                            Err(e) => Resp::WriteDone {
                                ok: false,
                                cmd: "absorb".into(),
                                output: vec![e],
                                ms,
                            },
                        };
                        let _ = ev.send(Msg::Git(resp));
                    });
                    None
                }
                Req::Copy(text) => {
                    let start = std::time::Instant::now();
                    let result = copy_to_clipboard(&text);
                    let ms = start.elapsed().as_millis() as u64;
                    Some(match result {
                        Ok(()) => Resp::WriteDone {
                            ok: true,
                            cmd: format!("copy {text}"),
                            output: Vec::new(),
                            ms,
                        },
                        Err(e) => Resp::WriteDone {
                            ok: false,
                            cmd: format!("copy {text}"),
                            output: vec![e],
                            ms,
                        },
                    })
                }
                Req::Ignore { pattern, local } => {
                    // The private file lives in the common directory, thus
                    // every worktree of the repository shares it.
                    let (file, name) = if local {
                        (common.join("info/exclude"), "exclude")
                    } else {
                        (worker_root.join(".gitignore"), ".gitignore")
                    };
                    let start = std::time::Instant::now();
                    let result = append_rule(&file, &pattern);
                    let ms = start.elapsed().as_millis() as u64;
                    Some(match result {
                        Ok(()) => Resp::WriteDone {
                            ok: true,
                            cmd: format!("{name} += {pattern}"),
                            output: Vec::new(),
                            ms,
                        },
                        Err(e) => Resp::WriteDone {
                            ok: false,
                            cmd: format!("{name} += {pattern}"),
                            output: vec![e.to_string()],
                            ms,
                        },
                    })
                }
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
                Req::LogChunk { .. }
                | Req::LogRefresh { .. }
                | Req::LogFilter(_)
                | Req::LogFirstParent(_) => None,
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

/// How long to wait between one quiet fetch and the next. LAZIER_FETCH_SECS
/// sets another number of seconds, and zero turns the fetching off.
fn fetch_every() -> Option<std::time::Duration> {
    let secs = std::env::var("LAZIER_FETCH_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(180);
    (secs > 0).then(|| std::time::Duration::from_secs(secs))
}

/// Fetch from the remote now and then, thus the count of commits you are
/// behind stays true. It writes nothing to the screen and it never opens a
/// window, because you did not ask for it.
fn spawn_auto_fetch(root: PathBuf, ev: Sender<Msg>) {
    let Some(every) = fetch_every() else { return };
    std::thread::spawn(move || {
        // A repository with no remote has nothing to fetch.
        let has_remote = Command::new("git")
            .arg("-C")
            .arg(&root)
            .arg("remote")
            .output()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false);
        if !has_remote {
            return;
        }
        loop {
            std::thread::sleep(every);
            let start = std::time::Instant::now();
            let out = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["fetch", "--quiet"])
                // A fetch that waits for a password would wait for ever.
                .env("GIT_TERMINAL_PROMPT", "0")
                .output();
            let ms = start.elapsed().as_millis() as u64;
            let ok = out.as_ref().map(|o| o.status.success()).unwrap_or(false);
            if !ok {
                let output = out.map(|o| take_lines(&o)).unwrap_or_default();
                if ev
                    .send(Msg::Git(Resp::Background { ok, cmd: "fetch".into(), output, ms }))
                    .is_err()
                {
                    return;
                }
                continue;
            }
            // The counts come from the refs, thus they are right now.
            if ev.send(Msg::Git(sync_state(&root))).is_err() {
                return;
            }
        }
    });
}

/// True for a command that talks to a remote. Those commands are slow.
pub fn is_network(args: &[String]) -> bool {
    matches!(args.first().map(String::as_str), Some("push" | "pull" | "fetch"))
}

/// Editors to try when git names one that is not on this machine. The
/// first ones are the easiest to leave.
const FALLBACK_EDITORS: &[&str] = &["nano", "micro", "hx", "helix", "nvim", "vim", "vi", "notepad"];

#[cfg(windows)]
const EXE_SUFFIXES: &[&str] = &["", ".exe", ".cmd", ".bat"];
#[cfg(not(windows))]
const EXE_SUFFIXES: &[&str] = &[""];

/// The editor to open a file with. Git already knows the answer: it reads
/// GIT_EDITOR, then core.editor, then VISUAL, then EDITOR. Ask git, thus
/// one setting controls this program and your other git commands. Git can
/// still name an editor that is not here, thus make sure of it first.
pub fn editor(root: &PathBuf) -> String {
    let named = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["var", "GIT_EDITOR"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    if on_path(&named) {
        return named;
    }
    FALLBACK_EDITORS.iter().find(|name| on_path(name)).unwrap_or(&"vi").to_string()
}

/// True when the first word of a command line names a program you can run.
/// The word can carry flags after it, thus only the first word counts.
fn on_path(cmd: &str) -> bool {
    let program = cmd.split_whitespace().next().unwrap_or_default().trim_matches('"');
    if program.is_empty() {
        return false;
    }
    // A name with a directory in it points at the file, not at the PATH.
    if program.contains('/') || program.contains('\\') {
        return std::path::Path::new(program).is_file();
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path)
        .any(|dir| EXE_SUFFIXES.iter().any(|end| dir.join(format!("{program}{end}")).is_file()))
}

/// The number of output lines that the command log keeps.
const LOG_LINES: usize = 12;

fn take_lines(out: &std::process::Output) -> Vec<String> {
    // Git writes progress to stderr, thus both streams matter.
    let text =
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    clean_lines(&text)
}

/// The lines of an output, ready for the command log. Git marks each line
/// that comes from the server with "remote:". The mark costs width and
/// says nothing, thus take it away. A line that is then empty says nothing
/// at all, thus drop it and keep the row for a line that does.
fn clean_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.strip_prefix("remote:").map_or(l, str::trim_start))
        .filter(|l| !l.trim().is_empty())
        .take(LOG_LINES)
        .map(str::to_string)
        .collect()
}

fn run_git(root: &PathBuf, args: &[String]) -> Resp {
    let cmd = format!("git {}", args.join(" "));
    let start = std::time::Instant::now();
    let result = Command::new("git")
        .arg("-C")
        .arg(root)
        // Never wait for a password. A background command has no terminal
        // to ask on, thus it must fail instead of hanging.
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(args)
        .output();
    let ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(out) => Resp::WriteDone { ok: out.status.success(), cmd, output: take_lines(&out), ms },
        Err(e) => Resp::WriteDone { ok: false, cmd, output: vec![e.to_string()], ms },
    }
}

/// Run a command line through the shell. The user types it after a colon.
fn run_shell(root: &PathBuf, line: &str) -> Resp {
    let start = std::time::Instant::now();
    // Each system has its own shell and its own flag for a command line.
    let (program, flag) = if cfg!(windows) { ("cmd", "/C") } else { ("sh", "-c") };
    let result = Command::new(program).arg(flag).arg(line).current_dir(root).output();
    let ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(out) => Resp::WriteDone {
            ok: out.status.success(),
            cmd: format!(": {line}"),
            output: take_lines(&out),
            ms,
        },
        Err(e) => {
            Resp::WriteDone { ok: false, cmd: format!(": {line}"), output: vec![e.to_string()], ms }
        }
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
            Resp::WriteDone { ok: true, cmd: format!("{cmd} ({verb})"), output: Vec::new(), ms }
        }
        Ok(out) => Resp::WriteDone { ok: false, cmd, output: take_lines(&out), ms },
        Err(e) => Resp::WriteDone { ok: false, cmd, output: vec![e], ms },
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
            "--format=%(refname:short)\t%(HEAD)\t%(upstream:track)\t%(committerdate:relative)\t%(refname)",
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
                remote: parts.next().unwrap_or("").starts_with("refs/remotes/"),
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

/// Who last changed each line of a file. The porcelain form is stable,
/// thus it is safer to read than the plain one.
fn blame(root: &PathBuf, path: &str) -> Vec<BlameLine> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["blame", "--line-porcelain", "-w", "--", path])
        .output();
    let Ok(out) = out else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut lines = Vec::new();
    let (mut id, mut author, mut date) = (String::new(), String::new(), String::new());
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("author ") {
            author = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("author-time ") {
            date = rest.parse::<u32>().map(ymd).unwrap_or_default();
        } else if let Some(rest) = line.strip_prefix('\t') {
            // A line that starts with a tab holds the text of the file.
            lines.push(BlameLine {
                id: id.chars().take(7).collect(),
                author: author.clone(),
                date: date.clone(),
                text: rest.to_string(),
            });
        } else if line.len() >= 40 && line.split(' ').next().is_some_and(|w| w.len() == 40) {
            id = line.split(' ').next().unwrap_or("").to_string();
        }
    }
    lines
}

// Convert epoch seconds to a calendar date, with no date library.
fn ymd(secs: u32) -> String {
    let z = (secs / 86400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The submodules and their state.
fn submodules(root: &PathBuf) -> Vec<SubmoduleEntry> {
    let out = Command::new("git").arg("-C").arg(root).args(["submodule", "status"]).output();
    let Ok(out) = out else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let state = line.chars().next().filter(|c| !c.is_ascii_alphanumeric()).unwrap_or(' ');
            let rest = if state == ' ' { line } else { &line[1..] };
            let mut p = rest.split_whitespace();
            let id = p.next()?.chars().take(7).collect();
            Some(SubmoduleEntry { id, path: p.next()?.to_string(), state })
        })
        .collect()
}

/// Put text on the clipboard. Each system has its own program for it.
fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(windows) {
        &[("clip", &[])]
    } else {
        &[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"]), ("xsel", &["-ib"])]
    };
    for (program, args) in candidates {
        let child = Command::new(program).args(*args).stdin(Stdio::piped()).spawn();
        if let Ok(mut child) = child {
            let ok = child
                .stdin
                .take()
                .map(|mut s| s.write_all(text.as_bytes()).is_ok())
                .unwrap_or(false);
            let _ = child.wait();
            if ok {
                return Ok(());
            }
        }
    }
    Err("no clipboard program was found".into())
}

/// The recent positions of HEAD. It is the way back from a mistake.
fn reflog(root: &PathBuf) -> Vec<ReflogEntry> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["reflog", "-n", "60", "--format=%h%x09%gd%x09%gs"])
        .output();
    let Ok(out) = out else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut p = line.split('\t');
            Some(ReflogEntry {
                id: p.next()?.to_string(),
                at: p.next()?.to_string(),
                what: p.next().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// List the worktrees. The output has one block for each worktree. A block
/// has a "worktree" line and often a "branch" line.
fn worktrees(root: &PathBuf) -> Vec<WorktreeEntry> {
    let out =
        Command::new("git").arg("-C").arg(root).args(["worktree", "list", "--porcelain"]).output();
    let Ok(out) = out else { return Vec::new() };
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    // Compare real paths. A symbolic link makes two names for one directory.
    let here = root.canonicalize().unwrap_or_else(|_| root.clone());
    let mut list = Vec::new();
    for block in text.split("\n\n") {
        let mut path = String::new();
        let mut branch = String::from("(detached)");
        let (mut locked, mut prunable) = (false, false);
        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = p.to_string();
            } else if let Some(b) = line.strip_prefix("branch ") {
                branch = b.trim_start_matches("refs/heads/").to_string();
            } else if line.starts_with("locked") {
                locked = true;
            } else if line.starts_with("prunable") {
                prunable = true;
            } else if line.starts_with("bare") {
                branch = "(bare)".into();
            }
        }
        if !path.is_empty() {
            let real = PathBuf::from(&path);
            let current = real.canonicalize().unwrap_or(real) == here;
            // Git always prints the main worktree first.
            let main = list.is_empty();
            list.push(WorktreeEntry { path, branch, current, main, locked, prunable });
        }
    }
    list
}

/// Make a short age from the words that git gives. "3 days ago" gives "3d".
fn short_age(text: &str) -> String {
    let mut words = text.split_whitespace();
    let Some(n) = words.next() else {
        return String::new();
    };
    let Some(unit) = words.next() else {
        return String::new();
    };
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
        Some((_, rest)) => rest
            .trim_start()
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap_or("")
            .parse()
            .unwrap_or(0),
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
    let upstream = run(&["rev-parse", "--abbrev-ref", "@{upstream}"])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    Resp::Sync { ahead, behind, unpushed, upstream }
}

fn display_diff(root: &PathBuf, target: &DiffTarget) -> (String, String) {
    // `diff --no-index` reports a difference with exit code one, thus the
    // output counts whenever there is any.
    let run = |args: &[&str]| match Command::new("git").arg("-C").arg(root).args(args).output() {
        Ok(out) if out.status.success() || !out.stdout.is_empty() => {
            String::from_utf8_lossy(&out.stdout).into_owned()
        }
        Ok(out) => String::from_utf8_lossy(&out.stderr).into_owned(),
        Err(e) => e.to_string(),
    };
    match target {
        // A file that git does not track has nothing to compare against,
        // thus compare it with an empty file. Every line is then an add.
        DiffTarget::WorktreeFile { path, untracked: true } => {
            (run(&["diff", "--no-index", "--", "/dev/null", path]), String::new())
        }
        // The work-tree diff is index-to-worktree. The hunk view applies
        // these hunks with `apply --cached`, thus the base is the index.
        DiffTarget::WorktreeFile { path, .. } => {
            (run(&["diff", "--", path]), run(&["diff", "--cached", "--", path]))
        }
        DiffTarget::Commit(id) => (run(&["show", "--stat", "--patch", id]), String::new()),
        DiffTarget::Range { from, to } => {
            (run(&["diff", "--stat", "--patch", &format!("{from}..{to}")]), String::new())
        }
        DiffTarget::Stash(i) => {
            (run(&["stash", "show", "--stat", "--patch", &format!("stash@{{{i}}}")]), String::new())
        }
    }
}

#[cfg(test)]
mod editor_tests {
    use super::on_path;

    #[test]
    fn finds_a_program_that_is_there() {
        let real = if cfg!(windows) { "cmd" } else { "sh" };
        assert!(on_path(real));
        // Flags after the name must not hide the program.
        assert!(on_path(&format!("{real} --wait")));
    }

    #[test]
    fn rejects_a_program_that_is_not_there() {
        assert!(!on_path("lazier-no-such-editor-9f3a"));
        assert!(!on_path(""));
        assert!(!on_path("   "));
    }

    #[test]
    fn a_name_with_a_directory_must_be_a_file() {
        assert!(!on_path("/lazier/no/such/editor"));
        assert!(on_path(if cfg!(windows) { r"C:\Windows\System32\cmd.exe" } else { "/bin/sh" }));
    }
}

#[cfg(test)]
mod log_tests {
    use super::clean_lines;

    /// The answer of a push carries the link that makes a pull request.
    /// The link must survive, thus the log can show it.
    #[test]
    fn a_push_keeps_the_link_and_drops_the_marks() {
        let out = "\
remote:
remote: Create a pull request for 'work' on GitHub by visiting:
remote:      https://github.com/maxmalkin/lazier/pull/new/work
remote:
To https://github.com/maxmalkin/lazier.git
 * [new branch]      work -> work
";
        assert_eq!(
            clean_lines(out),
            [
                "Create a pull request for 'work' on GitHub by visiting:",
                "https://github.com/maxmalkin/lazier/pull/new/work",
                "To https://github.com/maxmalkin/lazier.git",
                " * [new branch]      work -> work",
            ]
        );
    }

    #[test]
    fn a_line_that_is_not_from_the_server_keeps_its_shape() {
        assert_eq!(clean_lines("error: it did not work\n"), ["error: it did not work"]);
    }

    #[test]
    fn the_log_takes_only_the_first_lines() {
        let many: String = (0..40).map(|i| format!("line {i}\n")).collect();
        assert_eq!(clean_lines(&many).len(), super::LOG_LINES);
    }
}
