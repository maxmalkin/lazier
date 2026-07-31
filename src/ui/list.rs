//! This list renders only the rows in view. A function maps an index to a
//! row. The list length can be very large. Do not iterate the full range in
//! this file.
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
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
    // The focused panel has a green frame. The other frames are dim.
    let block = if focused {
        Block::bordered().title(title).border_style(Style::new().fg(Color::Green))
    } else {
        Block::bordered().title(title).border_style(Style::new().fg(Color::DarkGray))
    };
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible = inner.height as usize;
    if visible == 0 || len == 0 {
        return;
    }
    // Keep the selected row in view. After a scroll, the row stays at the
    // bottom edge.
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
