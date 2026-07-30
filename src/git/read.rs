//! All gix calls live in this file. When the gix API changes, correct only
//! this file.
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use super::{CommitEntry, FileEntry, Resp};
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

pub fn branches(repo: &gix::Repository) -> Option<Resp> {
    let current = repo.head_name().ok().flatten().map(|n| n.shorten().to_string());
    let mut names = Vec::new();
    if let Ok(platform) = repo.references()
        && let Ok(iter) = platform.local_branches()
    {
        for r in iter.filter_map(Result::ok) {
            names.push(r.name().shorten().to_string());
        }
    }
    names.sort();
    Some(Resp::Branches { current, names })
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
pub fn log_thread(shared: Arc<gix::ThreadSafeRepository>, rx: Receiver<usize>, ev: Sender<Msg>) {
    let repo = shared.to_thread_local();
    let mut walk = repo.head_id().ok().and_then(|id| id.ancestors().all().ok());
    for count in rx {
        let mut entries = Vec::with_capacity(count);
        let mut done = walk.is_none();
        if let Some(w) = walk.as_mut() {
            for _ in 0..count {
                match w.next() {
                    Some(Ok(info)) => {
                        let subject = info
                            .object()
                            .ok()
                            .and_then(|c| c.message().ok().map(|m| m.summary().to_string()))
                            .unwrap_or_default();
                        entries.push(CommitEntry {
                            id: info.id.to_hex_with_len(7).to_string(),
                            subject,
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
