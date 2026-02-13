use std::io;

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};
use ratatui::Terminal;

use super::input::cursor_to_row_col;
use super::state::{ChatState, CopyButton, MessageRole};

// ── Styles ────────────────────────────────────────────────────────────

const fn s_code() -> Style { Style::new().fg(Color::Green) }
const fn s_border() -> Style { Style::new().fg(Color::DarkGray) }
const fn s_btn() -> Style { Style::new().fg(Color::Yellow) }
const fn s_lang() -> Style {
    Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
}

// ── Text wrapping ─────────────────────────────────────────────────────

pub(crate) fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut start = 0;
        let chars: Vec<char> = paragraph.chars().collect();
        while start < chars.len() {
            let end = (start + width).min(chars.len());
            out.push(chars[start..end].iter().collect());
            start = end;
        }
    }
    out
}

pub(crate) fn chat_max_scroll(state: &ChatState, chat_area: Rect) -> usize {
    let lines = render_chat_lines(state, chat_area.width.saturating_sub(2) as usize, None, &mut Vec::new(), &mut Vec::new());
    let visible = chat_area.height.saturating_sub(2) as usize;
    lines.len().saturating_sub(visible.max(1))
}

// ── Chat line rendering ───────────────────────────────────────────────

/// Render chat into styled `Line`s.  Also fills `plain_lines` with one
/// plain-text string per visual line, guaranteed 1:1 with the returned
/// `Vec<Line>`, so selection offset math stays perfectly aligned.
pub(crate) fn render_chat_lines(
    state: &ChatState,
    width: usize,
    selection: Option<(usize, usize)>,
    copy_buttons: &mut Vec<CopyButton>,
    plain_lines: &mut Vec<String>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let content_width = width.saturating_sub(2).max(1);
    copy_buttons.clear();
    plain_lines.clear();

    let sel = selection.map(|(a, b)| (a.min(b), a.max(b)));
    let highlight = Style::default().bg(Color::DarkGray).fg(Color::White);

    let mut offset: usize = 0;

    for message in &state.messages {
        let (label, color) = match message.role {
            MessageRole::System => ("System", Color::DarkGray),
            MessageRole::User => ("You", Color::Gray),
            MessageRole::Assistant => ("Jose", Color::LightBlue),
        };

        // Header line
        let header_text = format!("● {}", label);
        let header_len = header_text.chars().count();
        let header_spans = build_selected_spans(
            &header_text, offset, sel,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
            highlight,
        );
        lines.push(Line::from(header_spans));
        plain_lines.push(header_text);
        offset += header_len + 1;

        let is_assistant = matches!(message.role, MessageRole::Assistant);
        let raw_lines: Vec<&str> = message.content.split('\n').collect();

        let mut i = 0;
        while i < raw_lines.len() {
            let raw = raw_lines[i];

            if is_assistant && raw.trim_start().starts_with("```") {
                // ── Fenced code block ─────────────────────────────
                let lang = raw.trim_start().trim_start_matches('`').trim();
                let mut code_content: Vec<&str> = Vec::new();
                i += 1;
                while i < raw_lines.len() && !raw_lines[i].trim_start().starts_with("```") {
                    code_content.push(raw_lines[i]);
                    i += 1;
                }
                if i < raw_lines.len() {
                    i += 1;
                }

                let code_text = code_content.join("\n");

                // Top border: ── lang ────────── ⎘  (fits within width)
                let btn_text = " ⎘";
                let btn_chars = btn_text.chars().count();
                let lang_display = if lang.is_empty() { String::new() } else { format!(" {} ", lang) };
                // "  ──" (4) + lang + fill + " " (1) + btn must fit in width
                let prefix_len = 4 + lang_display.chars().count();
                let fill = width.saturating_sub(prefix_len + 1 + btn_chars);
                let top_border = format!("  ──{}{} ", lang_display, "─".repeat(fill));
                let top_plain = format!("{}{}", top_border, btn_text);
                let top_plain_len = top_plain.chars().count();
                let line_idx = lines.len();

                let mut spans = Vec::new();
                if lang_display.is_empty() {
                    spans.push(Span::styled(top_border.clone(), s_border()));
                } else {
                    spans.push(Span::styled("  ──".to_string(), s_border()));
                    spans.push(Span::styled(lang_display.clone(), s_lang()));
                    spans.push(Span::styled(format!("{} ", "─".repeat(fill)), s_border()));
                }
                spans.push(Span::styled(btn_text.to_string(), s_btn()));
                lines.push(Line::from(spans));
                plain_lines.push(top_plain);
                offset += top_plain_len + 1;

                copy_buttons.push(CopyButton {
                    line: line_idx,
                    col_start: top_border.chars().count(),
                    content: code_text.clone(),
                });

                // Code body with left border
                for code_line in &code_content {
                    for wrapped in wrap_text(code_line, content_width.saturating_sub(4)) {
                        let line_text = format!("  │ {}", wrapped);
                        let line_len = line_text.chars().count();
                        let spans = build_selected_spans(&line_text, offset, sel, s_code(), highlight);
                        lines.push(Line::from(spans));
                        plain_lines.push(line_text);
                        offset += line_len + 1;
                    }
                }

                // Bottom border (fit within width)
                let bottom = format!("  └{}", "─".repeat(width.saturating_sub(3)));
                let bottom_len = bottom.chars().count();
                lines.push(Line::from(Span::styled(bottom.clone(), s_border())));
                plain_lines.push(bottom);
                offset += bottom_len + 1;
            } else if is_assistant {
                // ── Normal assistant line — parse inline `code` ────
                render_inline_code_line(
                    raw, &mut lines, &mut offset, content_width, sel, highlight, copy_buttons, plain_lines,
                );
                i += 1;
            } else {
                // Non-assistant: plain wrap
                for wrapped in wrap_text(raw, content_width) {
                    let line_text = format!("  {}", wrapped);
                    let line_len = line_text.chars().count();
                    let spans = build_selected_spans(&line_text, offset, sel, Style::default(), highlight);
                    lines.push(Line::from(spans));
                    plain_lines.push(line_text);
                    offset += line_len + 1;
                }
                i += 1;
            }
        }

        // Separator
        lines.push(Line::from(Span::styled("", Style::default())));
        plain_lines.push(String::new());
        offset += 1;
    }

    lines
}

/// Render a single assistant content line, detecting inline `code` spans.
#[allow(clippy::too_many_arguments)]
fn render_inline_code_line(
    raw: &str,
    lines: &mut Vec<Line<'static>>,
    offset: &mut usize,
    content_width: usize,
    sel: Option<(usize, usize)>,
    highlight: Style,
    copy_buttons: &mut Vec<CopyButton>,
    plain_lines: &mut Vec<String>,
) {
    let segments = parse_inline_code(raw);
    let has_code = segments.iter().any(|(_, is_code)| *is_code);

    if !has_code {
        for wrapped in wrap_text(raw, content_width) {
            let line_text = format!("  {}", wrapped);
            let line_len = line_text.chars().count();
            let spans = build_selected_spans(&line_text, *offset, sel, Style::default(), highlight);
            lines.push(Line::from(spans));
            plain_lines.push(line_text);
            *offset += line_len + 1;
        }
        return;
    }

    let line_idx = lines.len();
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled("  ".to_string(), Style::default()));
    let mut plain = String::from("  ");
    let mut col = 2usize;

    for (text, is_code) in &segments {
        if *is_code {
            spans.push(Span::styled(format!("`{}`", text), s_code()));
            let btn = " ⎘";
            let btn_col = col + text.chars().count() + 2;
            spans.push(Span::styled(btn.to_string(), s_btn()));
            copy_buttons.push(CopyButton {
                line: line_idx,
                col_start: btn_col,
                content: text.to_string(),
            });
            plain.push('`');
            plain.push_str(text);
            plain.push('`');
            plain.push_str(btn);
            col = btn_col + btn.chars().count();
        } else {
            spans.push(Span::styled(text.to_string(), Style::default()));
            plain.push_str(text);
            col += text.chars().count();
        }
    }

    lines.push(Line::from(spans));
    plain_lines.push(plain);
    *offset += col + 1;
}

/// Split text into (content, is_code) segments based on backtick delimiters.
fn parse_inline_code(text: &str) -> Vec<(String, bool)> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_code = false;
    let chars = text.chars();

    for ch in chars {
        if ch == '`' {
            if in_code {
                // Closing backtick
                segments.push((current.clone(), true));
                current.clear();
                in_code = false;
            } else {
                // Opening backtick
                if !current.is_empty() {
                    segments.push((current.clone(), false));
                    current.clear();
                }
                in_code = true;
            }
        } else {
            current.push(ch);
        }
    }

    // Leftover
    if !current.is_empty() {
        segments.push((current, in_code));
    }

    segments
}

// ── Selection-aware span builder ──────────────────────────────────────

pub(crate) fn build_selected_spans(
    text: &str,
    line_offset: usize,
    sel: Option<(usize, usize)>,
    normal_style: Style,
    highlight_style: Style,
) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();

    let Some((sel_start, sel_end)) = sel else {
        return vec![Span::styled(text.to_string(), normal_style)];
    };

    let line_end_offset = line_offset + len;
    if sel_end <= line_offset || sel_start >= line_end_offset {
        return vec![Span::styled(text.to_string(), normal_style)];
    }

    let local_start = sel_start.saturating_sub(line_offset).min(len);
    let local_end = sel_end.saturating_sub(line_offset).min(len);

    let mut spans = Vec::new();
    if local_start > 0 {
        spans.push(Span::styled(chars[..local_start].iter().collect::<String>(), normal_style));
    }
    spans.push(Span::styled(chars[local_start..local_end].iter().collect::<String>(), highlight_style));
    if local_end < len {
        spans.push(Span::styled(chars[local_end..].iter().collect::<String>(), normal_style));
    }
    spans
}

// ── draw_ui ───────────────────────────────────────────────────────────

pub(crate) fn draw_ui(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, state: &mut ChatState) -> Result<()> {
    let term_size = terminal.size()?;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(7),
            Constraint::Length(1),
        ])
        .split(term_size.into());

    let mut copy_buttons = Vec::new();
    let mut plain_lines = Vec::new();
    let chat_lines = render_chat_lines(state, chunks[0].width.saturating_sub(2) as usize, state.selection, &mut copy_buttons, &mut plain_lines);
    state.copy_buttons = copy_buttons;
    state.plain_lines = plain_lines;

    let chunks_copy = chunks.clone();
    terminal.draw(|frame| {
        let chunks = chunks_copy;
        let chat_len = chat_lines.len();
        let max_scroll = {
            let visible = chunks[0].height.saturating_sub(2) as usize;
            chat_len.saturating_sub(visible.max(1))
        };
        let scroll = if state.auto_follow {
            max_scroll
        } else {
            state.chat_scroll.min(max_scroll)
        } as u16;

        let chat = Paragraph::new(chat_lines)
            .block(
                Block::default()
                    .title(format!(" Chat ({}) ", state.model))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .scroll((scroll, 0));

        let chat_visible = chunks[0].height.saturating_sub(2) as usize;
        let mut chat_scrollbar_state = ScrollbarState::new(max_scroll)
            .viewport_content_length(chat_visible)
            .position(scroll as usize);
        let chat_scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .thumb_style(Style::default().fg(Color::Gray))
            .track_style(Style::default().fg(Color::DarkGray));

        let input_inner_width = chunks[1].width.saturating_sub(2) as usize;
        let mut wrapped_input = wrap_text(&state.input, input_inner_width.max(1));
        if wrapped_input.is_empty() {
            wrapped_input.push(String::new());
        }

        let (cursor_row, cursor_col) = cursor_to_row_col(&state.input, state.cursor_pos, input_inner_width.max(1));
        let input_visible_lines = chunks[1].height.saturating_sub(2) as usize;
        let input_total_lines = wrapped_input.len();

        let input_scroll = if cursor_row >= input_visible_lines {
            cursor_row.saturating_sub(input_visible_lines - 1)
        } else {
            0
        };
        let input_start = input_scroll.min(input_total_lines.saturating_sub(input_visible_lines.max(1)));
        let input_end = (input_start + input_visible_lines.max(1)).min(input_total_lines);
        let input_slice = wrapped_input[input_start..input_end].to_vec();
        let input_text = input_slice.join("\n");

        let input = Paragraph::new(input_text)
            .block(
                Block::default()
                    .title(" Input ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .wrap(Wrap { trim: false });

        let input_max_scroll = input_total_lines.saturating_sub(input_visible_lines.max(1));
        let mut input_scrollbar_state = ScrollbarState::new(input_max_scroll)
            .viewport_content_length(input_visible_lines)
            .position(input_start);
        let input_scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .thumb_style(Style::default().fg(Color::Gray))
            .track_style(Style::default().fg(Color::DarkGray));

        let hint = Paragraph::new(Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            Span::raw("=send  "),
            Span::styled("Drag", Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            Span::raw("=select  "),
            Span::styled("Ctrl+C", Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            Span::raw("=copy/exit  "),
            Span::styled("Esc", Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            Span::raw("=exit"),
        ]))
        .style(Style::default().fg(Color::DarkGray));

        frame.render_widget(chat, chunks[0]);
        frame.render_stateful_widget(chat_scrollbar, chunks[0], &mut chat_scrollbar_state);
        frame.render_widget(input, chunks[1]);
        frame.render_stateful_widget(input_scrollbar, chunks[1], &mut input_scrollbar_state);
        if cursor_row >= input_start && cursor_row < input_end {
            let visible_row = cursor_row - input_start;
            let max_col = input_slice
                .get(visible_row)
                .map(|line| line.chars().count())
                .unwrap_or(0);
            let col = cursor_col.min(max_col);
            let x = chunks[1].x + 1 + col as u16;
            let y = chunks[1].y + 1 + visible_row as u16;
            frame.set_cursor_position((x, y));
        }
        frame.render_widget(hint, chunks[2]);
    })?;

    Ok(())
}
