//! All gix calls live in this file. When the gix API changes, correct only
//! this file.
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use super::{CommitEntry, FileEntry, LogReq, Resp};
use crate::event::Msg;

pub fn status(repo: &gix::Repository) -> Option<Resp> {
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
    let mut files = Vec::new();
    if index_matches_head(repo) {
        for item in platform.into_index_worktree_iter(None).ok()?.filter_map(Result::ok) {
            files.push(FileEntry {
                mark: worktree_mark(&item),
                staged: false,
                path: item.rela_path().to_string(),
            });
        }
    } else {
        for item in platform.into_iter(None).ok()?.filter_map(Result::ok) {
            use gix::status::Item;
            let (mark, staged) = match &item {
                Item::TreeIndex(c) => {
                    use gix::diff::index::ChangeRef;
                    let m = match c {
                        ChangeRef::Addition { .. } => 'A',
                        ChangeRef::Deletion { .. } => 'D',
                        ChangeRef::Modification { .. } => 'M',
                        ChangeRef::Rewrite { .. } => 'R',
                    };
                    (m, true)
                }
                Item::IndexWorktree(i) => (worktree_mark(i), false),
            };
            files.push(FileEntry { mark, staged, path: item.location().to_string() });
        }
    }
    Some(Resp::Status(files))
}

// Compare the index cache-tree root with the HEAD tree. A valid and equal
// root means that the index has no staged changes.
fn index_matches_head(repo: &gix::Repository) -> bool {
    let Ok(commit) = repo.head_commit() else { return false };
    let Ok(head_tree) = commit.tree_id() else { return false };
    let Ok(index) = repo.index() else { return false };
    match index.tree() {
        Some(t) if t.num_entries.is_some() => t.id == head_tree,
        _ => false,
    }
}

fn worktree_mark(item: &gix::status::index_worktree::Item) -> char {
    use gix::status::index_worktree::iter::Summary as S;
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

/// This thread owns the ancestor walker for its full life. The walker
/// borrows the thread-local repository, thus both stay in this stack frame.
/// Each request pulls the next `count` commits from the walker.
pub fn log_thread(shared: Arc<gix::ThreadSafeRepository>, rx: Receiver<LogReq>, ev: Sender<Msg>) {
    let mut repo = shared.to_thread_local();
    // The cache keeps decoded delta bases. Without it, the walk decodes the
    // same base objects again for each commit.
    repo.object_cache_size(Some(16 * 1024 * 1024));
    let mut walk = repo.head_id().ok().and_then(|id| id.ancestors().sorting(gix::revision::walk::Sorting::ByCommitTime(Default::default())).all().ok());
    let mut graph: super::graph::Graph<gix::ObjectId> = super::graph::Graph::new();
    for req in rx {
        let count = match req {
            LogReq::Chunk(count) => count,
            // A reset starts the walk again from the new HEAD.
            LogReq::Reset => {
                walk = repo.head_id().ok().and_then(|id| id.ancestors().sorting(gix::revision::walk::Sorting::ByCommitTime(Default::default())).all().ok());
                graph = super::graph::Graph::new();
                continue;
            }
        };
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
                                c.message().ok().map(|m| m.summary().to_string()).unwrap_or_default(),
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
        if ev.send(Msg::Git(Resp::LogChunk { entries, done })).is_err() {
            break;
        }
    }
}
