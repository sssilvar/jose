use super::render::wrap_text;
use super::state::ChatState;

use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crossterm::event::KeyModifiers;

pub(crate) fn cursor_to_row_col(text: &str, cursor_pos: usize, width: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let pos = cursor_pos.min(chars.len());
    let width = width.max(1);

    let mut row = 0;
    let mut col = 0;
    let mut line_start = 0;

    for (i, &ch) in chars.iter().enumerate().take(pos) {
        if ch == '\n' {
            row += 1;
            col = 0;
            line_start = i + 1;
        } else {
            let line_pos = i - line_start;
            if line_pos > 0 && line_pos % width == 0 {
                row += 1;
            }
            col = line_pos % width;
        }
    }

    if pos > line_start {
        let line_pos = pos - line_start;
        col = line_pos % width;
        if line_pos > 0 && line_pos.is_multiple_of(width) && pos < chars.len() {
            row += 1;
            col = 0;
        }
    }

    (row, col)
}

pub(crate) fn row_col_to_cursor(text: &str, target_row: usize, target_col: usize, width: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let width = width.max(1);

    let mut row = 0;
    let mut line_start = 0;

    for (i, &ch) in chars.iter().enumerate() {
        if row == target_row {
            let line_pos = i - line_start;
            if line_pos == target_col {
                return i;
            }
        }

        if ch == '\n' {
            if row == target_row {
                return i.min(line_start + target_col);
            }
            row += 1;
            line_start = i + 1;
        } else {
            let line_pos = i - line_start;
            if line_pos > 0 && line_pos % width == 0 {
                if row == target_row {
                    return i.min(line_start + target_col);
                }
                row += 1;
                line_start = i;
            }
        }
    }

    if row == target_row {
        let line_len = chars.len() - line_start;
        return (line_start + target_col.min(line_len)).min(chars.len());
    }

    chars.len()
}

pub(crate) fn delete_prev_word(input: &mut String, cursor_pos: &mut usize) {
    if *cursor_pos == 0 {
        return;
    }
    let chars: Vec<char> = input.chars().collect();
    let pos = (*cursor_pos).min(chars.len());
    let mut start = pos;

    while start > 0 && chars[start - 1].is_whitespace() {
        start -= 1;
    }
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }

    let before: String = chars[..start].iter().collect();
    let after: String = chars[pos..].iter().collect();
    *input = format!("{}{}", before, after);
    *cursor_pos = start;
}

/// Insert a character at cursor position.
pub(crate) fn insert_char(state: &mut ChatState, ch: char) {
    let chars: Vec<char> = state.input.chars().collect();
    let pos = state.cursor_pos.min(chars.len());
    let before: String = chars[..pos].iter().collect();
    let after: String = chars[pos..].iter().collect();
    state.input = format!("{}{}{}", before, ch, after);
    state.cursor_pos = pos + 1;
}

/// Insert a newline at cursor position.
pub(crate) fn insert_newline(state: &mut ChatState) {
    let chars: Vec<char> = state.input.chars().collect();
    let pos = state.cursor_pos.min(chars.len());
    let before: String = chars[..pos].iter().collect();
    let after: String = chars[pos..].iter().collect();
    state.input = format!("{}\n{}", before, after);
    state.cursor_pos = pos + 1;
}

/// Compute layout chunks for the terminal.
pub(crate) fn compute_layout(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(7),
            Constraint::Length(1),
        ])
        .split(area)
}

/// Move cursor left, optionally jumping by word with Alt.
pub(crate) fn move_left(state: &mut ChatState, modifiers: KeyModifiers) {
    if modifiers.contains(KeyModifiers::ALT) {
        let chars: Vec<char> = state.input.chars().collect();
        let mut pos = state.cursor_pos.min(chars.len());
        while pos > 0 && chars[pos - 1].is_whitespace() { pos -= 1; }
        while pos > 0 && !chars[pos - 1].is_whitespace() { pos -= 1; }
        state.cursor_pos = pos;
    } else {
        state.cursor_pos = state.cursor_pos.saturating_sub(1);
    }
}

/// Move cursor right, optionally jumping by word with Alt.
pub(crate) fn move_right(state: &mut ChatState, modifiers: KeyModifiers) {
    let len = state.input.chars().count();
    if modifiers.contains(KeyModifiers::ALT) {
        let chars: Vec<char> = state.input.chars().collect();
        let mut pos = state.cursor_pos.min(chars.len());
        while pos < chars.len() && !chars[pos].is_whitespace() { pos += 1; }
        while pos < chars.len() && chars[pos].is_whitespace() { pos += 1; }
        state.cursor_pos = pos;
    } else {
        state.cursor_pos = (state.cursor_pos + 1).min(len);
    }
}

/// Move cursor up one visual row.
pub(crate) fn move_up(state: &mut ChatState, input_width: usize) {
    let w = input_width.max(1);
    let (row, col) = cursor_to_row_col(&state.input, state.cursor_pos, w);
    if row > 0 {
        state.cursor_pos = row_col_to_cursor(&state.input, row - 1, col, w);
    }
}

/// Move cursor down one visual row.
pub(crate) fn move_down(state: &mut ChatState, input_width: usize) {
    let w = input_width.max(1);
    let wrapped = wrap_text(&state.input, w);
    let (row, col) = cursor_to_row_col(&state.input, state.cursor_pos, w);
    if row + 1 < wrapped.len() {
        state.cursor_pos = row_col_to_cursor(&state.input, row + 1, col, w);
    } else {
        state.cursor_pos = state.input.chars().count();
    }
}

/// Delete character before cursor.
pub(crate) fn backspace(state: &mut ChatState, modifiers: KeyModifiers) {
    if modifiers.intersects(KeyModifiers::ALT | KeyModifiers::CONTROL | KeyModifiers::META) {
        delete_prev_word(&mut state.input, &mut state.cursor_pos);
    } else if state.cursor_pos > 0 {
        let chars: Vec<char> = state.input.chars().collect();
        let pos = state.cursor_pos.min(chars.len());
        let before: String = chars[..pos - 1].iter().collect();
        let after: String = chars[pos..].iter().collect();
        state.input = format!("{}{}", before, after);
        state.cursor_pos = pos - 1;
    }
}

/// Delete character at cursor.
pub(crate) fn delete(state: &mut ChatState, modifiers: KeyModifiers) {
    if modifiers.intersects(KeyModifiers::ALT | KeyModifiers::CONTROL | KeyModifiers::META) {
        delete_prev_word(&mut state.input, &mut state.cursor_pos);
    } else {
        let chars: Vec<char> = state.input.chars().collect();
        let pos = state.cursor_pos.min(chars.len());
        if pos < chars.len() {
            let before: String = chars[..pos].iter().collect();
            let after: String = chars[pos + 1..].iter().collect();
            state.input = format!("{}{}", before, after);
        }
    }
}
