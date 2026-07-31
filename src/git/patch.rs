//! Split a unified diff into a file header and hunks. Build a patch for one
//! hunk. `git apply --cached` consumes the patch.

/// Split one-file diff text. Return the file header and the hunk blocks.
/// Each hunk block starts at its `@@` line.
pub fn split_diff(text: &str) -> Option<(String, Vec<String>)> {
    let first_hunk = text.find("\n@@")? + 1;
    let header = text[..first_hunk].to_string();
    let mut hunks = Vec::new();
    let body = &text[first_hunk..];
    let mut starts: Vec<usize> = vec![0];
    let mut pos = 0;
    while let Some(next) = body[pos..].find("\n@@") {
        pos += next + 1;
        starts.push(pos);
    }
    for (i, &s) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(body.len());
        hunks.push(body[s..end].to_string());
    }
    Some((header, hunks))
}

/// Build a patch that contains the file header and one hunk.
pub fn hunk_patch(header: &str, hunk: &str) -> String {
    let mut patch = format!("{header}{hunk}");
    if !patch.ends_with('\n') {
        patch.push('\n');
    }
    patch
}

/// Make a hunk that holds only the selected changes. The index numbers
/// point to the body lines of the hunk, thus the `@@` line is not one of
/// them.
///
/// A change that the user did not select must not go away. An added line
/// that is not selected goes out of the hunk. A removed line that is not
/// selected becomes a context line, thus the line stays in the file.
pub fn subset_hunk(hunk: &str, selected: &[usize]) -> Option<String> {
    let mut lines = hunk.lines();
    let head = lines.next()?;
    let (old_start, new_start) = parse_header(head)?;

    let mut body = String::new();
    let (mut old_count, mut new_count) = (0u32, 0u32);
    for (i, line) in lines.enumerate() {
        let take = selected.contains(&i);
        let (kind, rest) = line.split_at(line.char_indices().next().map_or(0, |(_, c)| c.len_utf8()).min(line.len()));
        match (kind, take) {
            // Git marks a missing end-of-line with a backslash. Keep it.
            ("\\", _) => {
                body.push_str(line);
                body.push('\n');
                continue;
            }
            ("+", true) => {
                body.push_str(line);
                new_count += 1;
            }
            ("+", false) => continue,
            ("-", true) => {
                body.push_str(line);
                old_count += 1;
            }
            // The line stays in the file, thus it becomes context.
            ("-", false) => {
                body.push(' ');
                body.push_str(rest);
                old_count += 1;
                new_count += 1;
            }
            _ => {
                body.push_str(line);
                old_count += 1;
                new_count += 1;
            }
        }
        body.push('\n');
    }
    // A hunk with no change does nothing.
    if old_count == new_count && !body.contains("\n+") && !body.starts_with('+') && !body.contains("\n-") && !body.starts_with('-') {
        return None;
    }
    Some(format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@\n{body}"))
}

// Read the two line numbers out of a "@@ -a,b +c,d @@" line.
fn parse_header(head: &str) -> Option<(u32, u32)> {
    let mid = head.strip_prefix("@@ ")?.split(" @@").next()?;
    let mut parts = mid.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let first = |s: &str| s.split(',').next().unwrap_or("").parse::<u32>().ok();
    Some((first(old)?, first(new)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn git(dir: &std::path::Path, args: &[&str]) -> String {
        let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn apply_cached(dir: &std::path::Path, patch: &str, reverse: bool) {
        let mut args = vec!["-C", dir.to_str().unwrap(), "apply", "--cached"];
        if reverse {
            args.push("-R");
        }
        let mut child = Command::new("git").args(&args).stdin(Stdio::piped()).spawn().unwrap();
        use std::io::Write;
        child.stdin.take().unwrap().write_all(patch.as_bytes()).unwrap();
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn subset_keeps_only_the_selected_change() {
        // Two added lines. The user takes the second one only.
        let hunk = "@@ -1,2 +1,4 @@\n line one\n+added A\n+added B\n line two\n";
        let out = subset_hunk(hunk, &[2]).unwrap();
        assert_eq!(out, "@@ -1,2 +1,3 @@\n line one\n+added B\n line two\n");
    }

    #[test]
    fn a_removal_that_is_not_selected_becomes_context() {
        // Two removed lines. The user takes the first one only. The second
        // line must stay in the file.
        let hunk = "@@ -1,3 +1,1 @@\n keep\n-gone A\n-gone B\n";
        let out = subset_hunk(hunk, &[1]).unwrap();
        assert_eq!(out, "@@ -1,3 +1,2 @@\n keep\n-gone A\n gone B\n");
    }

    #[test]
    fn a_subset_with_no_change_gives_nothing() {
        let hunk = "@@ -1,2 +1,3 @@\n line one\n+added A\n line two\n";
        assert!(subset_hunk(hunk, &[]).is_none());
    }

    // Stage one changed line out of three in the same hunk. Git must accept
    // the patch. The index must hold that one change and no other.
    #[test]
    fn one_line_goes_to_the_index() {
        let dir = std::env::temp_dir().join(format!("lazier-line-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "t@t"]);
        git(&dir, &["config", "user.name", "t"]);

        let start: Vec<String> = (1..=6).map(|i| format!("line {i}")).collect();
        std::fs::write(dir.join("f.txt"), start.join("\n") + "\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-qm", "init"]);

        // Change three lines that sit close together, thus one hunk holds
        // all of them.
        let mut now = start.clone();
        now[1] = "line 2 CHANGED".into();
        now[2] = "line 3 CHANGED".into();
        now[3] = "line 4 CHANGED".into();
        std::fs::write(dir.join("f.txt"), now.join("\n") + "\n").unwrap();

        let diff = git(&dir, &["diff", "--", "f.txt"]);
        let (header, hunks) = split_diff(&diff).unwrap();
        assert_eq!(hunks.len(), 1, "expected one hunk:\n{diff}");

        // The body holds: context, -2, -3, -4, +2, +3, +4, context.
        // Take the removal of line 3 and the addition of line 3.
        let body: Vec<&str> = hunks[0].lines().skip(1).collect();
        let minus3 = body.iter().position(|l| *l == "-line 3").unwrap();
        let plus3 = body.iter().position(|l| *l == "+line 3 CHANGED").unwrap();
        let subset = subset_hunk(&hunks[0], &[minus3, plus3]).unwrap();
        apply_cached(&dir, &hunk_patch(&header, &subset), false);

        let staged = git(&dir, &["diff", "--cached", "--", "f.txt"]);
        assert!(staged.contains("+line 3 CHANGED"), "staged:\n{staged}");
        assert!(!staged.contains("+line 2 CHANGED"), "line 2 must stay out:\n{staged}");
        assert!(!staged.contains("+line 4 CHANGED"), "line 4 must stay out:\n{staged}");
        // The other two changes are still in the work tree.
        let rest = git(&dir, &["diff", "--", "f.txt"]);
        assert!(rest.contains("+line 2 CHANGED") && rest.contains("+line 4 CHANGED"), "rest:\n{rest}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // This is the money path. Stage one hunk of two, then reverse it. The
    // index must return to its start state.
    #[test]
    fn hunk_round_trip() {
        let dir = std::env::temp_dir().join(format!("lazier-patch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "t@t"]);
        git(&dir, &["config", "user.name", "t"]);

        let lines: Vec<String> = (1..=20).map(|i| format!("line {i}")).collect();
        std::fs::write(dir.join("f.txt"), lines.join("\n") + "\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-qm", "init"]);
        let clean_index = git(&dir, &["write-tree"]);

        // Change line 2 and line 18. The distance makes two hunks.
        let mut changed = lines.clone();
        changed[1] = "line 2 CHANGED".into();
        changed[17] = "line 18 CHANGED".into();
        std::fs::write(dir.join("f.txt"), changed.join("\n") + "\n").unwrap();

        let diff = git(&dir, &["diff", "--", "f.txt"]);
        let (header, hunks) = split_diff(&diff).unwrap();
        assert_eq!(hunks.len(), 2, "expected two hunks:\n{diff}");

        // Stage only the first hunk.
        let patch = hunk_patch(&header, &hunks[0]);
        apply_cached(&dir, &patch, false);
        let cached = git(&dir, &["diff", "--cached", "--", "f.txt"]);
        assert!(cached.contains("line 2 CHANGED"));
        assert!(!cached.contains("line 18 CHANGED"));

        // Reverse it. The index must equal the initial tree again.
        apply_cached(&dir, &patch, true);
        assert_eq!(git(&dir, &["write-tree"]), clean_index);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
