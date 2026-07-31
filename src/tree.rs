//! Build a file tree from flat status paths. The output is the flat list of
//! visible rows. Collapsed directories hide their children.
use std::collections::HashSet;

use crate::git::FileEntry;

pub struct TreeRow {
    pub depth: u8,
    pub name: String,
    /// A directory row has its full path here.
    pub dir: Option<String>,
    /// A file row points to its entry in the status list.
    pub file: Option<usize>,
}

pub fn build(files: &[FileEntry], collapsed: &HashSet<String>) -> Vec<TreeRow> {
    let mut order: Vec<usize> = (0..files.len()).collect();
    order.sort_by(|a, b| files[*a].path.cmp(&files[*b].path));

    let mut rows = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    for i in order {
        let path = &files[i].path;
        let comps: Vec<&str> = path.split('/').collect();
        let (dirs, name) = comps.split_at(comps.len() - 1);
        // Find the common prefix with the open directory stack.
        let mut common = 0;
        while common < stack.len() && common < dirs.len() && stack[common] == dirs[common] {
            common += 1;
        }
        stack.truncate(common);
        for d in &dirs[common..] {
            stack.push(d.to_string());
            if !hidden(&stack[..stack.len() - 1], collapsed) {
                rows.push(TreeRow {
                    depth: (stack.len() - 1) as u8,
                    name: d.to_string(),
                    dir: Some(stack.join("/")),
                    file: None,
                });
            }
        }
        if !hidden(&stack, collapsed) {
            rows.push(TreeRow { depth: stack.len() as u8, name: name[0].to_string(), dir: None, file: Some(i) });
        }
    }
    rows
}

// A row is hidden when any directory above it is collapsed.
fn hidden(stack: &[String], collapsed: &HashSet<String>) -> bool {
    let mut path = String::new();
    for d in stack {
        if !path.is_empty() {
            path.push('/');
        }
        path.push_str(d);
        if collapsed.contains(&path) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(paths: &[&str]) -> Vec<FileEntry> {
        paths.iter().map(|p| FileEntry { mark: 'M', staged: false, path: p.to_string() }).collect()
    }

    #[test]
    fn nests_and_collapses() {
        let files = entries(&["a/b/f1", "a/f2", "top.txt"]);
        let rows = build(&files, &HashSet::new());
        let names: Vec<(&str, u8)> = rows.iter().map(|r| (r.name.as_str(), r.depth)).collect();
        assert_eq!(names, [("a", 0), ("b", 1), ("f1", 2), ("f2", 1), ("top.txt", 0)]);

        let collapsed: HashSet<String> = ["a/b".to_string()].into();
        let rows = build(&files, &collapsed);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        // The b row stays. Its children go away.
        assert_eq!(names, ["a", "b", "f2", "top.txt"]);

        let collapsed: HashSet<String> = ["a".to_string()].into();
        let rows = build(&files, &collapsed);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["a", "top.txt"]);
    }
}
