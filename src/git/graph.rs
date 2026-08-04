//! Lane engine for the commit graph. Each commit gets one text row of
//! box-drawing glyphs. The engine holds one lane for each open branch line.
//! The caller feeds commits in walk order and stores the returned row.
//! This module must stay free of gix types.

pub struct Graph<T: PartialEq + Clone> {
    lanes: Vec<Option<T>>,
}

impl<T: PartialEq + Clone> Graph<T> {
    pub fn new() -> Self {
        Self { lanes: Vec::new() }
    }

    /// Add one commit. Return its graph row.
    pub fn row(&mut self, id: &T, parents: &[T]) -> String {
        // The commit takes the first lane that waits for it. A new head
        // opens a new lane.
        let col = match self.lanes.iter().position(|l| l.as_ref() == Some(id)) {
            Some(c) => c,
            None => {
                self.lanes.push(Some(id.clone()));
                self.lanes.len() - 1
            }
        };
        // Other lanes that wait for the same commit join it on this row.
        let joins: Vec<usize> = self
            .lanes
            .iter()
            .enumerate()
            .filter(|(i, l)| *i != col && l.as_ref() == Some(id))
            .map(|(i, _)| i)
            .collect();
        let old_active: Vec<bool> = self.lanes.iter().map(Option::is_some).collect();

        // The first parent continues in this lane. No parent closes it.
        self.lanes[col] = parents.first().cloned();

        // Each other parent is a merge source. Point to its lane when one
        // already waits for it. Else open a lane in the first free slot.
        let mut merges: Vec<usize> = Vec::new();
        let mut opened: Vec<usize> = Vec::new();
        for p in parents.iter().skip(1) {
            if let Some(i) = self.lanes.iter().position(|l| l.as_ref() == Some(p)) {
                merges.push(i);
            } else {
                let i = match self.lanes.iter().position(|l| l.is_none()) {
                    Some(i) => i,
                    None => {
                        self.lanes.push(None);
                        self.lanes.len() - 1
                    }
                };
                self.lanes[i] = Some(p.clone());
                opened.push(i);
            }
        }

        let width = self.lanes.len().max(old_active.len());
        let mut cells: Vec<char> = vec![' '; 2 * width - 1];
        for (i, active) in old_active.iter().enumerate() {
            if *active {
                cells[2 * i] = '│';
            }
        }
        // Horizontal edges come first. The end glyphs overwrite them.
        for &target in joins.iter().chain(&merges).chain(&opened) {
            let (lo, hi) = (col.min(target), col.max(target));
            for pos in (2 * lo + 1)..(2 * hi) {
                cells[pos] = if pos % 2 == 0 && cells[pos] == '│' {
                    '┼'
                } else if pos % 2 == 1 {
                    '─'
                } else {
                    cells[pos]
                };
            }
        }
        for &j in &joins {
            cells[2 * j] = if j > col { '╯' } else { '╰' };
        }
        for &m in &merges {
            cells[2 * m] = if m > col { '┤' } else { '├' };
        }
        for &o in &opened {
            cells[2 * o] = if o > col { '╮' } else { '╭' };
        }
        cells[2 * col] = if parents.len() > 1 { '◉' } else { '●' };

        // The joined lanes close after this row.
        for j in joins {
            self.lanes[j] = None;
        }
        while self.lanes.last() == Some(&None) {
            self.lanes.pop();
        }
        cells.into_iter().collect::<String>().trim_end().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_chain() {
        let mut g: Graph<u32> = Graph::new();
        assert_eq!(g.row(&3, &[2]), "●");
        assert_eq!(g.row(&2, &[1]), "●");
        assert_eq!(g.row(&1, &[]), "●");
    }

    #[test]
    fn branch_and_merge() {
        // M merges A and B. Both come from C.
        let (m, a, b, c) = (4u32, 3, 2, 1);
        let mut g: Graph<u32> = Graph::new();
        assert_eq!(g.row(&m, &[a, b]), "◉─╮");
        assert_eq!(g.row(&a, &[c]), "● │");
        assert_eq!(g.row(&b, &[c]), "│ ●");
        assert_eq!(g.row(&c, &[]), "●─╯");
    }

    #[test]
    fn merge_into_tracked_lane() {
        // M2 merges A2 and B. M1 merges A1 and B2. B2 and B share a line.
        // Feed: M2(a2,b) A2(a1) B(b0) A1(a0) ... checks the ┤ glyph appears
        // when a merge points to a lane that already exists.
        let mut g: Graph<u32> = Graph::new();
        assert_eq!(g.row(&10, &[8, 6]), "◉─╮");
        assert_eq!(g.row(&8, &[7, 6]), "◉─┤");
        assert_eq!(g.row(&7, &[5]), "● │");
        assert_eq!(g.row(&6, &[5]), "│ ●");
        assert_eq!(g.row(&5, &[]), "●─╯");
    }

    #[test]
    fn two_roots() {
        // Two histories with no common commit.
        let mut g: Graph<u32> = Graph::new();
        assert_eq!(g.row(&4, &[2]), "●");
        assert_eq!(g.row(&3, &[1]), "│ ●");
        assert_eq!(g.row(&2, &[]), "● │");
        assert_eq!(g.row(&1, &[]), "  ●");
    }
}
