//! Find the words that differ between a removed line and the added line
//! that follows it. The diff pane then marks only those words, thus a
//! small change in a long line is easy to see.

/// The part of each line that changed, as a range of byte positions. The
/// text before and after that range is the same in both lines.
pub struct WordSpan {
    pub old: (usize, usize),
    pub new: (usize, usize),
}

// A word is a run of letters and digits. Every other character stands
// alone, thus punctuation marks a boundary.
fn words(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        let wordy = c.is_alphanumeric() || c == '_';
        match (wordy, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, i));
                out.push((i, i + c.len_utf8()));
                start = None;
            }
            (false, None) => out.push((i, i + c.len_utf8())),
            (true, Some(_)) => {}
        }
    }
    if let Some(s) = start {
        out.push((s, text.len()));
    }
    out
}

/// Compare two lines. The result names the middle part of each line that
/// is not the same. None means the lines share no useful boundary, thus
/// the caller should mark the whole line.
pub fn changed_span(old: &str, new: &str) -> Option<WordSpan> {
    if old == new {
        return None;
    }
    let (ow, nw) = (words(old), words(new));
    // Count the words that match at the start, then at the end.
    let mut head = 0;
    while head < ow.len()
        && head < nw.len()
        && old[ow[head].0..ow[head].1] == new[nw[head].0..nw[head].1]
    {
        head += 1;
    }
    let mut tail = 0;
    while tail < ow.len() - head
        && tail < nw.len() - head
        && old[ow[ow.len() - 1 - tail].0..ow[ow.len() - 1 - tail].1]
            == new[nw[nw.len() - 1 - tail].0..nw[nw.len() - 1 - tail].1]
    {
        tail += 1;
    }
    // Nothing matches at either end, thus the whole line changed.
    if head == 0 && tail == 0 {
        return None;
    }
    let o_start = ow.get(head).map(|w| w.0).unwrap_or(old.len());
    let o_end = if tail == 0 { old.len() } else { ow[ow.len() - tail].0 };
    let n_start = nw.get(head).map(|w| w.0).unwrap_or(new.len());
    let n_end = if tail == 0 { new.len() } else { nw[nw.len() - tail].0 };
    Some(WordSpan { old: (o_start, o_end.max(o_start)), new: (n_start, n_end.max(n_start)) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(old: &str, new: &str) -> Option<(String, String)> {
        changed_span(old, new)
            .map(|s| (old[s.old.0..s.old.1].to_string(), new[s.new.0..s.new.1].to_string()))
    }

    #[test]
    fn finds_one_word_in_the_middle() {
        assert_eq!(
            span("let value = compute(a, b);", "let value = derive(a, b);"),
            Some(("compute".into(), "derive".into()))
        );
    }

    #[test]
    fn finds_a_change_at_the_end() {
        assert_eq!(span("count = 1", "count = 20"), Some(("1".into(), "20".into())));
    }

    #[test]
    fn finds_a_change_at_the_start() {
        assert_eq!(
            span("old_name.run()", "new_name.run()"),
            Some(("old_name".into(), "new_name".into()))
        );
    }

    #[test]
    fn finds_added_text() {
        // Nothing goes away, thus the old side is empty.
        assert_eq!(span("fn run()", "fn run() -> bool"), Some(("".into(), " -> bool".into())));
    }

    // Two lines with nothing in common give nothing, thus the caller marks
    // the whole line instead of a confusing part of it.
    #[test]
    fn gives_nothing_when_the_lines_share_nothing() {
        assert!(span("alpha beta", "gamma delta").is_none());
    }

    #[test]
    fn gives_nothing_for_equal_lines() {
        assert!(span("same", "same").is_none());
    }
}
