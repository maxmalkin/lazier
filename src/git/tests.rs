//! Regression tests against real repositories. The unit tests elsewhere
//! cover pure logic. These make a repository on the disk and read it back,
//! thus a change in gix that alters what a status reports cannot pass
//! without notice.
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{Resp, read};

/// A repository in a temporary directory. It goes away when the test ends,
/// and also when the test panics.
struct Repo {
    dir: PathBuf,
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Repo {
    /// Make an empty repository with one commit in it.
    fn new() -> Repo {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let n = COUNT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("lazier-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("make the directory");
        let repo = Repo { dir };
        repo.git(&["init", "--quiet"]);
        // A commit needs an author. A signature needs a key that a test
        // machine does not have, thus turn signing off.
        repo.git(&["config", "user.name", "Lazier Test"]);
        repo.git(&["config", "user.email", "test@example.com"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo.git(&["config", "tag.gpgsign", "false"]);
        repo.write("start.txt", "first\n");
        repo.git(&["add", "start.txt"]);
        repo.commit("start");
        repo
    }

    /// Run a git command. It must succeed, thus a broken test says so at
    /// the step that broke.
    fn git(&self, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn write(&self, name: &str, text: &str) {
        let path = self.dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("make the parent directory");
        }
        std::fs::write(path, text).expect("write the file");
    }

    fn remove(&self, name: &str) {
        std::fs::remove_file(self.dir.join(name)).expect("remove the file");
    }

    fn commit(&self, message: &str) {
        self.git(&["commit", "--quiet", "-m", message]);
    }

    fn open(&self) -> gix::Repository {
        gix::open(&self.dir).expect("open with gix")
    }

    /// The status, in the two-column form of `git status --short`. The rows
    /// are sorted, thus a test does not depend on the order of the walk.
    fn status(&self) -> Vec<String> {
        let Some(Resp::Status(files)) = read::status(&self.open()) else {
            panic!("the status gave nothing");
        };
        let mut rows: Vec<String> =
            files.iter().map(|f| format!("{}{} {}", f.index, f.work, f.path)).collect();
        rows.sort();
        rows
    }

    /// The status of the named paths only.
    fn status_paths(&self, paths: &[&str]) -> Vec<String> {
        let want: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
        let Some(Resp::StatusPaths { scanned, files }) = read::status_paths(&self.open(), &want)
        else {
            panic!("the status gave nothing");
        };
        assert_eq!(scanned, want, "the answer must say what it looked at");
        let mut rows: Vec<String> =
            files.iter().map(|f| format!("{}{} {}", f.index, f.work, f.path)).collect();
        rows.sort();
        rows
    }
}

#[test]
fn a_clean_repository_has_no_rows() {
    let repo = Repo::new();
    assert!(repo.status().is_empty());
}

/// gix calls an untracked file "added", the same word it uses for a staged
/// new file. Only the variant of the item tells them apart. A wrong answer
/// here made a delete run `git rm` on a file that git does not track.
#[test]
fn an_untracked_file_is_not_a_staged_file() {
    let repo = Repo::new();
    repo.write("new.txt", "hello\n");
    // Nothing is staged, thus the index column stays empty.
    assert_eq!(repo.status(), [" ? new.txt"]);
}

#[test]
fn a_changed_file_shows_in_the_work_tree_column() {
    let repo = Repo::new();
    repo.write("start.txt", "second\n");
    assert_eq!(repo.status(), [" M start.txt"]);
}

#[test]
fn a_staged_file_shows_in_the_index_column() {
    let repo = Repo::new();
    repo.write("new.txt", "hello\n");
    repo.git(&["add", "new.txt"]);
    assert_eq!(repo.status(), ["A  new.txt"]);
}

/// A file can carry a staged change and a work-tree change at once. It
/// must take one row with two marks, never two rows.
#[test]
fn one_file_with_two_changes_takes_one_row() {
    let repo = Repo::new();
    repo.write("start.txt", "second\n");
    repo.git(&["add", "start.txt"]);
    repo.write("start.txt", "third\n");
    assert_eq!(repo.status(), ["MM start.txt"]);
}

#[test]
fn a_removed_file_shows_as_removed() {
    let repo = Repo::new();
    repo.remove("start.txt");
    assert_eq!(repo.status(), [" D start.txt"]);
}

/// The index cache-tree shortcut skips the walk of the HEAD tree when the
/// index equals it. A work-tree change must still be found on that path.
#[test]
fn the_cache_tree_shortcut_still_finds_a_work_tree_change() {
    let repo = Repo::new();
    // A fresh commit leaves the index equal to the HEAD tree.
    repo.write("start.txt", "second\n");
    assert_eq!(repo.status(), [" M start.txt"]);
    // A staged change makes the index differ, thus the other path runs.
    repo.git(&["add", "start.txt"]);
    assert_eq!(repo.status(), ["M  start.txt"]);
}

/// The work-tree watcher names the paths that changed. A scan of those
/// paths must report them and leave every other path alone.
#[test]
fn a_scan_of_one_path_reports_only_that_path() {
    let repo = Repo::new();
    repo.write("one.txt", "one\n");
    repo.write("two.txt", "two\n");
    assert_eq!(repo.status_paths(&["one.txt"]), [" ? one.txt"]);
    assert_eq!(repo.status_paths(&["two.txt"]), [" ? two.txt"]);
}

/// The incremental scan takes the place of what the full scan said about a
/// path. The two must agree on every path, or a file would change its row
/// only because of which scan found it.
#[test]
fn a_scan_of_a_path_agrees_with_the_full_scan() {
    let repo = Repo::new();
    repo.write("src/a.txt", "a\n");
    repo.write("src/b.txt", "b\n");
    repo.write("start.txt", "second\n");
    repo.git(&["add", "start.txt"]);
    let full = repo.status();
    for path in ["src", "start.txt"] {
        let rows = repo.status_paths(&[path]);
        let want: Vec<String> = full.iter().filter(|r| r[3..].starts_with(path)).cloned().collect();
        assert_eq!(rows, want, "the scan of {path} must agree with the full scan");
    }
}

/// A path that is named and has no change must come back with no row. The
/// caller drops what it knew about that path, thus a file that went back
/// to its old content stops showing a change.
#[test]
fn a_scan_of_a_clean_path_reports_no_row() {
    let repo = Repo::new();
    assert!(repo.status_paths(&["start.txt"]).is_empty());
}

/// The tags once came from a git subprocess. That cost 38 MB on a large
/// repository. They now come from the open gix repository.
#[test]
fn a_tag_points_at_its_commit() {
    let repo = Repo::new();
    // This machine can force an annotated tag, thus always make one.
    repo.git(&["tag", "-a", "v1.0", "-m", "v1.0"]);
    let id = repo.git(&["rev-parse", "--short=7", "HEAD"]);
    let Some(Resp::Tags(map)) = read::tags(&repo.open()) else { panic!("no tags") };
    assert_eq!(map.get(&id).map(Vec::as_slice), Some(["v1.0".to_string()].as_slice()));
}

#[test]
fn a_repository_with_no_tag_gives_an_empty_map() {
    let repo = Repo::new();
    let Some(Resp::Tags(map)) = read::tags(&repo.open()) else { panic!("no tags") };
    assert!(map.is_empty());
}

#[test]
fn the_head_name_is_the_branch_name() {
    let repo = Repo::new();
    repo.git(&["branch", "-m", "work"]);
    assert_eq!(read::head_name(&repo.open()).as_deref(), Some("work"));
}

#[test]
fn a_detached_head_has_no_branch_name() {
    let repo = Repo::new();
    let id = repo.git(&["rev-parse", "HEAD"]);
    repo.git(&["checkout", "--quiet", &id]);
    assert_eq!(read::head_name(&repo.open()), None);
}

#[test]
fn a_stash_shows_its_message() {
    let repo = Repo::new();
    repo.write("start.txt", "second\n");
    repo.git(&["stash", "push", "--quiet", "-m", "keep this"]);
    let Some(Resp::Stashes(list)) = read::stashes(&repo.open()) else { panic!("no stashes") };
    assert_eq!(list.len(), 1);
    assert!(list[0].contains("keep this"), "the row was {:?}", list[0]);
    // The work tree went back to the committed content.
    assert!(repo.status().is_empty());
}

#[test]
fn no_stash_gives_an_empty_list() {
    let repo = Repo::new();
    let Some(Resp::Stashes(list)) = read::stashes(&repo.open()) else { panic!("no stashes") };
    assert!(list.is_empty());
}
