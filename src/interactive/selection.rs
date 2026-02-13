use std::time::{Duration, Instant};

use ratatui::layout::Rect;

use crossterm::event::{MouseButton, MouseEventKind};

use crate::clipboard::copy_to_clipboard;

use super::state::ChatState;

/// Map screen (row, col) to an offset into the flat chat plain text.
pub(crate) fn screen_to_chat_offset(
    row: u16,
    col: u16,
    chat_area: Rect,
    scroll: u16,
    plain_lines: &[String],
) -> Option<usize> {
    let inner_y = chat_area.y + 1;
    let inner_x = chat_area.x + 1;
    let inner_h = chat_area.height.saturating_sub(2);
    let inner_w = chat_area.width.saturating_sub(2);

    if row < inner_y || row >= inner_y + inner_h || col < inner_x || col >= inner_x + inner_w {
        return None;
    }

    let visual_row = (row - inner_y) as usize + scroll as usize;
    let visual_col = (col - inner_x) as usize;

    let mut offset = 0;
    for (i, line) in plain_lines.iter().enumerate() {
        if i == visual_row {
            let clamped_col = visual_col.min(line.chars().count());
            return Some(offset + clamped_col);
        }
        offset += line.chars().count() + 1;
    }
    Some(offset.saturating_sub(1))
}

/// Extract selected substring from plain lines.
pub(crate) fn extract_selection(plain_lines: &[String], sel: (usize, usize)) -> String {
    let flat: String = plain_lines.join("\n");
    let chars: Vec<char> = flat.chars().collect();
    let lo = sel.0.min(sel.1).min(chars.len());
    let hi = sel.0.max(sel.1).min(chars.len());
    chars[lo..hi].iter().collect()
}

/// Find word boundaries at offset position.
pub(crate) fn word_bounds_at(plain_lines: &[String], pos: usize) -> (usize, usize) {
    let flat: String = plain_lines.join("\n");
    let chars: Vec<char> = flat.chars().collect();
    let pos = pos.min(chars.len());

    let mut start = pos;
    while start > 0 && !chars[start - 1].is_whitespace() { start -= 1; }
    let mut end = pos;
    while end < chars.len() && !chars[end].is_whitespace() { end += 1; }
    (start, end)
}

/// Handle all mouse events. Returns Ok(true) if the event was consumed.
pub(crate) fn handle_mouse(
    state: &mut ChatState,
    kind: MouseEventKind,
    row: u16,
    column: u16,
    chat_area: Rect,
    scroll: u16,
    max_scroll: usize,
) {
    match kind {
        MouseEventKind::ScrollUp => {
            if state.auto_follow {
                state.auto_follow = false;
                state.chat_scroll = max_scroll;
            }
            state.chat_scroll = state.chat_scroll.saturating_sub(3);
        }
        MouseEventKind::ScrollDown => {
            if !state.auto_follow {
                state.chat_scroll = (state.chat_scroll + 3).min(max_scroll);
                if state.chat_scroll >= max_scroll {
                    state.auto_follow = true;
                }
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            // Check copy button hit first
            let inner_y = chat_area.y + 1;
            let inner_x = chat_area.x + 1;
            let visual_line = (row.saturating_sub(inner_y)) as usize + scroll as usize;
            let visual_col = column.saturating_sub(inner_x) as usize;

            let mut hit_btn = false;
            for btn in &state.copy_buttons {
                let btn_len = " ⎘".chars().count();
                if btn.line == visual_line
                    && visual_col >= btn.col_start
                    && visual_col < btn.col_start + btn_len
                {
                    let _ = copy_to_clipboard(&btn.content);
                    hit_btn = true;
                    break;
                }
            }

            if !hit_btn {
                if let Some(off) = screen_to_chat_offset(row, column, chat_area, scroll, &state.plain_lines) {
                    let now = Instant::now();
                    let is_double = state.last_click.is_some_and(|(t, r, c)| {
                        now.duration_since(t) < Duration::from_millis(400)
                            && r == row
                            && c == column
                    });

                    if is_double {
                        let (ws, we) = word_bounds_at(&state.plain_lines, off);
                        state.selection = Some((ws, we));
                        state.drag_anchor = None;
                        state.last_click = None;
                    } else {
                        state.drag_anchor = Some(off);
                        state.selection = Some((off, off));
                        state.last_click = Some((now, row, column));
                    }
                }
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(anchor) = state.drag_anchor {
                if let Some(off) = screen_to_chat_offset(row, column, chat_area, scroll, &state.plain_lines) {
                    state.selection = Some((anchor, off));
                }
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            state.drag_anchor = None;
        }
        _ => {}
    }
}
