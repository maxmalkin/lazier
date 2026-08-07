//! All gix calls live in this file. When the gix API changes, correct only
//! this file.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use super::{CommitEntry, FileEntry, LogReq, Resp};
use crate::event::Msg;

/// Look at the given paths only, and say what was looked at. A walk of a
/// large work tree costs a lot, thus naming the paths saves nearly all of
/// it when a few files changed.
pub fn status_paths(repo: &gix::Repository, paths: &[String]) -> Option<Resp> {
    let files = scan(repo, paths.iter().map(|p| p.as_str().into()).collect())?;
    Some(Resp::StatusPaths { scanned: paths.to_vec(), files })
}

pub fn status(repo: &gix::Repository) -> Option<Resp> {
    Some(Resp::Status(scan(repo, Vec::new())?))
}

/// The state of every path, or of the named paths when the list is not
/// empty. An empty list means the whole work tree.
fn scan(repo: &gix::Repository, patterns: Vec<gix::bstr::BString>) -> Option<Vec<FileEntry>> {
    // Do not track renames. Rename tracking reads blob content, which is
    // costly and can cause network fetches in a partial clone.
    let platform = repo
        .status(gix::progress::Discard)
        .ok()?
        .tree_index_track_renames(gix::status::tree_index::TrackRenames::Disabled)
        .index_worktree_rewrites(None);
    // The tree-index comparison walks the full HEAD tree. This is costly on
    // a large repository. Skip it when the index cache-tree shows that the
    // index is equal to the HEAD tree. Then no staged changes can exist.
    // Collect both sides for each path. A file can have a staged change and
    // a work-tree change at the same time, thus it needs one row with two
    // marks, not two rows.
    let mut marks: BTreeMap<String, (char, char)> = BTreeMap::new();
    if index_matches_head(repo) {
        for item in platform.into_index_worktree_iter(patterns).ok()?.filter_map(Result::ok) {
            marks.entry(item.rela_path().to_string()).or_insert((' ', ' ')).1 =
                worktree_mark(&item);
        }
    } else {
        for item in platform.into_iter(patterns).ok()?.filter_map(Result::ok) {
            use gix::status::Item;
            let path = item.location().to_string();
            let slot = marks.entry(path).or_insert((' ', ' '));
            match &item {
                Item::TreeIndex(c) => {
                    use gix::diff::index::ChangeRef;
                    slot.0 = match c {
                        ChangeRef::Addition { .. } => 'A',
                        ChangeRef::Deletion { .. } => 'D',
                        ChangeRef::Modification { .. } => 'M',
                        ChangeRef::Rewrite { .. } => 'R',
                    };
                }
                Item::IndexWorktree(i) => slot.1 = worktree_mark(i),
            }
        }
    }
    Some(marks.into_iter().map(|(path, (index, work))| FileEntry { index, work, path }).collect())
}

// Compare the index cache-tree root with the HEAD tree. A valid and equal
// root means that the index has no staged changes.
fn index_matches_head(repo: &gix::Repository) -> bool {
    let Ok(commit) = repo.head_commit() else {
        return false;
    };
    let Ok(head_tree) = commit.tree_id() else {
        return false;
    };
    let Ok(index) = repo.index() else {
        return false;
    };
    match index.tree() {
        Some(t) if t.num_entries.is_some() => t.id == head_tree,
        _ => false,
    }
}

fn worktree_mark(item: &gix::status::index_worktree::Item) -> char {
    use gix::status::index_worktree::Item;
    use gix::status::index_worktree::iter::Summary as S;
    // The dirwalk finds files that git does not track. Their summary says
    // "added", thus the variant is the only way to know they are new.
    if matches!(item, Item::DirectoryContents { .. }) {
        return '?';
    }
    match item.summary() {
        Some(S::Removed) => 'D',
        Some(S::Added) | Some(S::IntentToAdd) => 'A',
        Some(S::Modified) => 'M',
        Some(S::TypeChange) => 'T',
        Some(S::Renamed) => 'R',
        Some(S::Copied) => 'C',
        Some(S::Conflict) => 'U',
        None => '?',
    }
}

/// The name of the branch at HEAD. A detached HEAD gives None.
pub fn head_name(repo: &gix::Repository) -> Option<String> {
    repo.head_name().ok().flatten().map(|n| n.shorten().to_string())
}

/// The tags of each commit, by the short id of the commit.
///
/// A separate `git for-each-ref` would open its own object database to
/// peel the tags, which costs a lot of memory on a big repository. The
/// database here is open already.
pub fn tags(repo: &gix::Repository) -> Option<Resp> {
    let mut map: std::collections::HashMap<String, Vec<String>> = Default::default();
    let platform = repo.references().ok()?;
    for r in platform.tags().ok()?.filter_map(Result::ok) {
        let name = r.name().shorten().to_string();
        // An annotated tag points at a tag object. Follow it to the commit.
        let mut r = r;
        if let Ok(id) = r.peel_to_id() {
            map.entry(id.to_hex_with_len(7).to_string()).or_default().push(name);
        }
    }
    Some(Resp::Tags(map))
}

pub fn stashes(repo: &gix::Repository) -> Option<Resp> {
    let mut out = Vec::new();
    if let Ok(r) = repo.find_reference("refs/stash")
        && let Ok(Some(iter)) = r.log_iter().rev()
    {
        for (i, line) in iter.filter_map(Result::ok).enumerate() {
            out.push(format!("stash@{{{i}}}: {}", line.message));
        }
    }
    Some(Resp::Stashes(out))
}

/// Find the commits whose message holds the text. Git searches the whole
/// history, thus this finds more than the part that is in memory.
fn search(root: &std::path::Path, text: &str) -> Vec<CommitEntry> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "log",
            "-n",
            "500",
            "--fixed-strings",
            "--regexp-ignore-case",
            &format!("--grep={text}"),
            "--format=%h%x09%an%x09%ct%x09%s",
        ])
        .output();
    let Ok(out) = out else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut p = line.split('\t');
            let hex = p.next()?;
            let mut id = [b'0'; 7];
            let bytes = hex.as_bytes();
            id[..7.min(bytes.len())].copy_from_slice(&bytes[..7.min(bytes.len())]);
            Some(CommitEntry {
                id,
                graph: "".into(),
                author: p.next().unwrap_or("").into(),
                time: p.next().unwrap_or("0").parse().unwrap_or(0),
                subject: p.next().unwrap_or("").into(),
            })
        })
        .collect()
}

/// Walk the history from HEAD, the newest commit first. `first_parent`
/// follows only the first parent
/// of each commit, thus a merge hides the branch that came into it and the
/// list shows the main line alone.
fn start_walk(repo: &gix::Repository, first_parent: bool) -> Option<gix::revision::Walk<'_>> {
    repo.head_id().ok().and_then(|id| {
        let mut walk =
            id.ancestors().sorting(gix::revision::walk::Sorting::ByCommitTime(Default::default()));
        if first_parent {
            walk = walk.first_parent_only();
        }
        walk.all().ok()
    })
}

/// This thread owns the ancestor walker for its full life. The walker
/// borrows the thread-local repository, thus both stay in this stack frame.
/// Each request pulls the next `count` commits from the walker.
pub fn log_thread(
    shared: Arc<gix::ThreadSafeRepository>,
    root: std::path::PathBuf,
    rx: Receiver<LogReq>,
    ev: Sender<Msg>,
) {
    let mut repo = shared.to_thread_local();
    // The cache keeps decoded delta bases. Without it, the walk decodes the
    // same base objects again for each commit.
    repo.object_cache_size(Some(16 * 1024 * 1024));
    let mut head = repo.head_id().ok().map(|id| id.detach());
    let mut first_parent = false;
    let mut walk = start_walk(&repo, first_parent);
    let mut graph: super::graph::Graph<gix::ObjectId> = super::graph::Graph::new();
    // False until the first chunk goes out. The first refresh must fill the
    // panel, even though HEAD has not moved since the thread started.
    let mut sent_any = false;
    let mut filter: Option<String> = None;
    for req in rx {
        let mut replace = false;
        let count = match req {
            // A search runs over the whole history, thus git does it. The
            // results are not next to each other, so they carry no graph.
            LogReq::Filter(text) => {
                filter = text;
                let entries = match &filter {
                    Some(text) => search(&root, text),
                    None => Vec::new(),
                };
                if filter.is_some() {
                    let _ = ev.send(Msg::Git(Resp::LogReplace { entries, done: true }));
                    continue;
                }
                // No filter any more: walk the history from HEAD again.
                walk = start_walk(&repo, first_parent);
                graph = super::graph::Graph::new();
                replace = true;
                100
            }
            LogReq::FirstParent(on) => {
                first_parent = on;
                // A search does not follow parents, thus it does not change.
                if filter.is_some() {
                    continue;
                }
                walk = start_walk(&repo, first_parent);
                graph = super::graph::Graph::new();
                replace = true;
                100
            }
            LogReq::Chunk(count) if filter.is_some() => {
                // A filtered list is complete already.
                let _ = ev.send(Msg::Git(Resp::LogChunk { entries: Vec::new(), done: true }));
                let _ = count;
                continue;
            }
            LogReq::Chunk(count) => count,
            // A filtered list does not follow HEAD, thus a refresh runs the
            // search again.
            LogReq::Refresh(_) if filter.is_some() => {
                let entries = search(&root, filter.as_deref().unwrap_or(""));
                let _ = ev.send(Msg::Git(Resp::LogReplace { entries, done: true }));
                continue;
            }
            LogReq::Refresh(count) => {
                let now = repo.head_id().ok().map(|id| id.detach());
                // The list is still correct when HEAD did not move. A walk
                // of a large history is costly, thus skip it.
                if now == head && sent_any {
                    continue;
                }
                if now != head {
                    head = now;
                    walk = start_walk(&repo, first_parent);
                    graph = super::graph::Graph::new();
                }
                replace = true;
                count
            }
        };
        sent_any = true;
        let mut entries = Vec::with_capacity(count);
        let mut done = walk.is_none();
        if let Some(w) = walk.as_mut() {
            for _ in 0..count {
                match w.next() {
                    Some(Ok(info)) => {
                        let parents: Vec<gix::ObjectId> = info.parent_ids.iter().cloned().collect();
                        let row = graph.row(&info.id, &parents);
                        let (subject, author, time) = match info.object() {
                            Ok(c) => (
                                c.message()
                                    .ok()
                                    .map(|m| m.summary().to_string())
                                    .unwrap_or_default(),
                                c.author().ok().map(|a| a.name.to_string()).unwrap_or_default(),
                                c.time().ok().map(|t| t.seconds.max(0) as u32).unwrap_or(0),
                            ),
                            Err(_) => (String::new(), String::new(), 0),
                        };
                        let mut id = [b'0'; 7];
                        let hex = info.id.to_hex_with_len(7).to_string();
                        id.copy_from_slice(&hex.as_bytes()[..7]);
                        entries.push(CommitEntry {
                            id,
                            graph: row.into(),
                            subject: subject.into(),
                            author: author.into(),
                            time,
                        });
                    }
                    Some(Err(_)) => continue,
                    None => {
                        done = true;
                        break;
                    }
                }
            }
        }
        let resp = if replace {
            Resp::LogReplace { entries, done }
        } else {
            Resp::LogChunk { entries, done }
        };
        if ev.send(Msg::Git(resp)).is_err() {
            break;
        }
    }
}
