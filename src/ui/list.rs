//! This list renders only the rows in view. A function maps an index to a
//! row. The list length can be very large. Do not iterate the full range in
//! this file.
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Scrollbar, ScrollbarOrientation, ScrollbarState};

pub fn render(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    focused: bool,
    selected: usize,
    len: usize,
    row: &dyn Fn(usize) -> Line<'static>,
) {
    let border = if focused { Color::Green } else { Color::DarkGray };
    let mut block = Block::bordered()
        .title(Span::styled(title.to_string(), Style::new().fg(border).add_modifier(Modifier::BOLD)))
        .border_style(Style::new().fg(border));
    // The count sits in the bottom border, as lazygit does it.
    if len > 0 {
        block = block.title_bottom(
            Line::styled(
                format!("{} of {}", selected + 1, len),
                Style::new().fg(border),
            )
            .right_aligned(),
        );
    }
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
        let rect = Rect { y: inner.y + i as u16, height: 1, ..inner };
        frame.render_widget(row(idx), rect);
        // The selected row gets a bar across the full width. The text keeps
        // its own colors.
        if idx == selected {
            let bg = if focused { Color::Blue } else { Color::Rgb(48, 48, 56) };
            frame.buffer_mut().set_style(rect, Style::new().bg(bg));
        }
    }
    // A scrollbar appears only when the rows do not all fit.
    if len > visible {
        let mut state = ScrollbarState::new(len).position(selected);
        frame.render_stateful_widget(
            // Only the thumb shows. A track would hide the border.
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(None)
                .thumb_symbol("┃")
                .thumb_style(Style::new().fg(border)),
            area,
            &mut state,
        );
    }
}
