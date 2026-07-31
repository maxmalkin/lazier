//! Interactive rebase. This module holds the todo list model and the
//! detection of a stopped rebase. It calls no git command.
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq)]
pub enum TodoAction {
    Pick,
    Reword,
    Edit,
    Squash,
    Fixup,
    Drop,
}

impl TodoAction {
    pub fn word(self) -> &'static str {
        match self {
            TodoAction::Pick => "pick",
            TodoAction::Reword => "reword",
            TodoAction::Edit => "edit",
            TodoAction::Squash => "squash",
            TodoAction::Fixup => "fixup",
            TodoAction::Drop => "drop",
        }
    }
}

pub struct TodoItem {
    pub action: TodoAction,
    pub id: String,
    pub subject: String,
}

/// Make the text of the git todo file. The list comes in display order, the
/// newest commit first. The git todo file needs the oldest commit first.
pub fn serialize(items: &[TodoItem]) -> String {
    let mut out = String::new();
    for item in items.iter().rev() {
        out.push_str(item.action.word());
        out.push(' ');
        out.push_str(&item.id);
        out.push(' ');
        out.push_str(&item.subject);
        out.push('\n');
    }
    out
}

/// Quote a string for a shell command line. Git runs the sequence editor
/// through the shell, thus a path with a space must have quotes.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

pub struct RebaseInfo {
    pub step: usize,
    pub total: usize,
}

fn num(path: PathBuf) -> Option<usize> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Report the state of a rebase that stopped. A merge rebase counts with
/// msgnum and end. An apply rebase counts with next and last.
pub fn detect(git_dir: &Path) -> Option<RebaseInfo> {
    let merge = git_dir.join("rebase-merge");
    let apply = git_dir.join("rebase-apply");
    let (step, total) = if merge.is_dir() {
        (num(merge.join("msgnum")), num(merge.join("end")))
    } else if apply.is_dir() {
        (num(apply.join("next")), num(apply.join("last")))
    } else {
        return None;
    };
    Some(RebaseInfo { step: step.unwrap_or(0), total: total.unwrap_or(0) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(action: TodoAction, id: &str) -> TodoItem {
        TodoItem { action, id: id.into(), subject: format!("subject of {id}") }
    }

    // The list is newest first on screen. The file must be oldest first,
    // else git replays the commits in the wrong order.
    #[test]
    fn serialize_reverses_order() {
        let items = vec![
            item(TodoAction::Fixup, "ccc"),
            item(TodoAction::Drop, "bbb"),
            item(TodoAction::Pick, "aaa"),
        ];
        assert_eq!(
            serialize(&items),
            "pick aaa subject of aaa\n\
             drop bbb subject of bbb\n\
             fixup ccc subject of ccc\n"
        );
    }

    #[test]
    fn quotes_paths_with_spaces() {
        assert_eq!(sh_quote("/a b/lazier"), "'/a b/lazier'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn detects_stopped_rebase() {
        let dir = std::env::temp_dir().join(format!("lazier-rebase-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(detect(&dir).is_none());
        std::fs::create_dir_all(dir.join("rebase-merge")).unwrap();
        std::fs::write(dir.join("rebase-merge/msgnum"), "3\n").unwrap();
        std::fs::write(dir.join("rebase-merge/end"), "7\n").unwrap();
        let info = detect(&dir).unwrap();
        assert_eq!((info.step, info.total), (3, 7));
        std::fs::remove_dir_all(dir.join("rebase-merge")).unwrap();
        assert!(detect(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
