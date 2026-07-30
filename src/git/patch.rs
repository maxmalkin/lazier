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
