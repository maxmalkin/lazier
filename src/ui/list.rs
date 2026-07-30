//! Virtualized list: renders only the rows in view, fed by an index->row fn.
//! Phase 2 hands this lens in the hundreds of thousands — nothing here may
//! ever iterate 0..len.
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Block;
use ratatui::Frame;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    focused: bool,
    selected: usize,
    len: usize,
    row: &dyn Fn(usize) -> Line<'static>,
) {
    let block = Block::bordered().title(title);
    let block = if focused { block.border_style(Style::new().green()) } else { block };
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible = inner.height as usize;
    if visible == 0 || len == 0 {
        return;
    }
    // keep the selected row in view, pinned to the bottom once scrolled
    let offset = selected
        .saturating_sub(visible - 1)
        .min(len.saturating_sub(visible));
    for (i, idx) in (offset..len.min(offset + visible)).enumerate() {
        let mut line = row(idx);
        if idx == selected && focused {
            line = line.style(Style::new().reversed());
        }
        frame.render_widget(line, Rect { y: inner.y + i as u16, height: 1, ..inner });
    }
}
