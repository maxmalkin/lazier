//! Send each staged change to the commit that last touched those lines.
//!
//! For each hunk that waits in the index, blame tells which commit last
//! wrote the lines it changes. The hunks are grouped by that commit, one
//! fixup commit is made for each group, and a rebase folds them all in.
//!
//! Only a commit that no remote has yet can take a change, thus published
//! history never moves.
use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use super::patch;

/// One group: the commit that takes the changes, and the patch to apply.
struct Group {
    target: String,
    patch: String,
    hunks: usize,
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out =
        Command::new("git").arg("-C").arg(root).args(args).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn git_stdin(root: &Path, args: &[&str], input: &str) -> Result<(), String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    child
        .stdin
        .take()
        .ok_or("no input to the command")?
        .write_all(input.as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// The first line and the number of lines that a hunk covers on the old
/// side. This is the range of the hunk, context lines and all.
fn old_range(hunk: &str) -> Option<(u32, u32)> {
    let head = hunk.lines().next()?;
    let mid = head.strip_prefix("@@ ")?.split(" @@").next()?;
    let old = mid.split_whitespace().next()?.strip_prefix('-')?;
    let mut parts = old.split(',');
    let start: u32 = parts.next()?.parse().ok()?;
    let count: u32 = parts.next().unwrap_or("1").parse().unwrap_or(1);
    Some((start, count))
}

/// The old lines that the hunk really changes. Context lines are not in
/// the list, because the commit that wrote them is not the one to fix.
/// A hunk that only adds lines gives the line above the new text.
fn changed_lines(hunk: &str) -> Vec<u32> {
    let Some((start, _)) = old_range(hunk) else {
        return Vec::new();
    };
    let mut line = start;
    let mut out = Vec::new();
    // Where the first new text sits, for a hunk that removes nothing.
    let mut first_add = None;
    for text in hunk.lines().skip(1) {
        match text.as_bytes().first() {
            Some(b'-') => {
                out.push(line);
                line += 1;
            }
            Some(b'+') => {
                if first_add.is_none() {
                    first_add = Some(line);
                }
            }
            // A context line, or the note about a missing end of line.
            Some(b'\\') => {}
            _ => line += 1,
        }
    }
    if out.is_empty() {
        // Nothing goes away, thus take the line above the new text.
        out.push(first_add.unwrap_or(start).saturating_sub(1).max(1));
    }
    out
}

/// The commit that last wrote most of the given lines. Only the lines the
/// hunk changes count, thus the context around them cannot outvote them.
fn blame_target(root: &Path, file: &str, lines: &[u32]) -> Option<String> {
    let (&lo, &hi) = (lines.iter().min()?, lines.iter().max()?);
    let range = format!("{lo},{hi}");
    let out = git(root, &["blame", "-L", &range, "--porcelain", "HEAD", "--", file]).ok()?;
    // A header names the commit, the old line, and the line in the file.
    let mut by_line: HashMap<u32, String> = HashMap::new();
    for text in out.lines() {
        let mut p = text.split(' ');
        let (Some(id), Some(_), Some(final_line)) = (p.next(), p.next(), p.next()) else {
            continue;
        };
        if id.len() == 40
            && id.chars().all(|c| c.is_ascii_hexdigit())
            && let Ok(n) = final_line.parse::<u32>()
        {
            by_line.insert(n, id.to_string());
        }
    }
    let mut votes: HashMap<&String, usize> = HashMap::new();
    for l in lines {
        if let Some(id) = by_line.get(l) {
            *votes.entry(id).or_default() += 1;
        }
    }
    votes.into_iter().max_by_key(|(_, n)| *n).map(|(id, _)| id.clone())
}

/// Group the staged hunks by the commit that should take them.
fn plan(root: &Path, staged: &str, may_take: &dyn Fn(&str) -> bool) -> (Vec<Group>, usize) {
    let mut groups: HashMap<String, Group> = HashMap::new();
    let mut skipped = 0usize;
    // The diff holds one block for each file.
    for block in split_files(staged) {
        let Some(file) = file_of(&block) else {
            continue;
        };
        let Some((header, hunks)) = patch::split_diff(&block) else {
            continue;
        };
        for hunk in hunks {
            let lines = changed_lines(&hunk);
            if lines.is_empty() {
                skipped += 1;
                continue;
            }
            let target = blame_target(root, &file, &lines).filter(|id| may_take(id));
            let Some(target) = target else {
                skipped += 1;
                continue;
            };
            let g = groups.entry(target.clone()).or_insert_with(|| Group {
                target,
                patch: String::new(),
                hunks: 0,
            });
            // Each hunk carries the header of its own file.
            g.patch.push_str(&patch::hunk_patch(&header, &hunk));
            g.hunks += 1;
        }
    }
    (groups.into_values().collect(), skipped)
}

// Split a diff into one text for each file.
fn split_files(diff: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in diff.lines() {
        if line.starts_with("diff --git ") && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

// The name of the file that a diff block is about.
fn file_of(block: &str) -> Option<String> {
    block.lines().find_map(|l| l.strip_prefix("+++ b/").map(str::to_string))
}

/// Do the whole job. The result is a line for the command log.
pub fn run(root: &Path, may_take: &dyn Fn(&str) -> bool) -> Result<Vec<String>, String> {
    let staged = git(root, &["diff", "--cached"])?;
    if staged.trim().is_empty() {
        return Err("stage the changes you want to send back first".into());
    }
    let (groups, skipped) = plan(root, &staged, may_take);
    if groups.is_empty() {
        return Err("no change belongs to a commit that is still yours to change".into());
    }

    // Start from a clean index. The work tree keeps every change, thus
    // nothing is lost if a step fails.
    git(root, &["reset", "-q"])?;
    let mut made = 0usize;
    let mut notes = Vec::new();
    for g in &groups {
        // The three-way form still applies when earlier commits of this
        // run moved the lines.
        if let Err(e) = git_stdin(root, &["apply", "--cached", "--3way"], &g.patch) {
            notes.push(format!("could not apply the changes for {}: {e}", &g.target[..7]));
            continue;
        }
        match git(root, &["commit", "-q", &format!("--fixup={}", g.target)]) {
            Ok(_) => made += 1,
            Err(e) => notes.push(format!("could not commit for {}: {e}", &g.target[..7])),
        }
    }
    if made == 0 {
        return Err(notes.pop().unwrap_or_else(|| "nothing was absorbed".into()));
    }

    // Rebase from the parent of the oldest commit that took a change. The
    // oldest one has the most commits between it and HEAD.
    let mut oldest = (0u32, String::new());
    for g in &groups {
        let count = git(root, &["rev-list", "--count", &format!("{}..HEAD", g.target)])
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0);
        if count >= oldest.0 {
            oldest = (count, g.target.clone());
        }
    }
    let base = format!("{}^", oldest.1);
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rebase", "--autosquash", "--autostash", &base])
        .env("GIT_SEQUENCE_EDITOR", "true")
        .env("GIT_EDITOR", "true")
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        notes.insert(0, "the fixup commits are there, but the rebase stopped".into());
        notes.push(
            String::from_utf8_lossy(&out.stderr).lines().take(3).collect::<Vec<_>>().join(" "),
        );
        return Err(notes.join("; "));
    }

    let total: usize = groups.iter().map(|g| g.hunks).sum();
    let mut summary = vec![format!("{total} changes went back into {made} commits")];
    if skipped > 0 {
        summary.push(format!(
            "{skipped} changes stayed, because no commit of yours holds those lines"
        ));
    }
    summary.extend(notes);
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_old_range_of_a_hunk() {
        assert_eq!(old_range("@@ -12,5 +12,7 @@\n ctx\n"), Some((12, 5)));
        // No count means one line.
        assert_eq!(old_range("@@ -3 +3,2 @@\n ctx\n"), Some((3, 1)));
        // A hunk that only adds lines has a count of zero.
        assert_eq!(old_range("@@ -7,0 +8,3 @@\n+a\n"), Some((7, 0)));
    }

    // Only the lines that change may vote. Context lines belong to older
    // commits and would win by number alone.
    #[test]
    fn only_the_changed_lines_count() {
        let hunk = "@@ -1,3 +1,3 @@\n ctx one\n-old two\n+new two\n ctx three\n";
        assert_eq!(changed_lines(hunk), vec![2]);
    }

    #[test]
    fn several_removed_lines_all_count() {
        let hunk = "@@ -10,4 +10,3 @@\n ctx\n-a\n-b\n+c\n ctx\n";
        assert_eq!(changed_lines(hunk), vec![11, 12]);
    }

    #[test]
    fn a_hunk_that_only_adds_takes_the_line_above() {
        let hunk = "@@ -5,2 +5,3 @@\n ctx five\n+added\n ctx six\n";
        assert_eq!(changed_lines(hunk), vec![5]);
    }

    #[test]
    fn splits_a_diff_by_file() {
        let diff = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n\
                    diff --git a/y b/y\n--- a/y\n+++ b/y\n@@ -1 +1 @@\n-c\n+d\n";
        let parts = split_files(diff);
        assert_eq!(parts.len(), 2);
        assert_eq!(file_of(&parts[0]).as_deref(), Some("x"));
        assert_eq!(file_of(&parts[1]).as_deref(), Some("y"));
    }
}
