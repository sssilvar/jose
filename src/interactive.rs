use std::io;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyboardEnhancementFlags, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};
use ratatui::Terminal;

use crate::chatgpt::call_chatgpt_interactive_with_history;
use crate::config::Config;

enum MessageRole {
    System,
    User,
    Assistant,
}

struct ChatMessage {
    role: MessageRole,
    content: String,
}

struct ChatState {
    model: String,
    session_id: String,
    turns: Vec<(String, String)>,
    input: String,
    cursor_pos: usize,
    messages: Vec<ChatMessage>,
    chat_scroll: usize,
    auto_follow: bool,
}

impl ChatState {
    fn new(model: String) -> Self {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        Self {
            model,
            session_id: format!("jose-{}-{}", std::process::id(), millis),
            turns: Vec::new(),
            input: String::new(),
            cursor_pos: 0,
            messages: vec![
                ChatMessage {
                    role: MessageRole::System,
                    content: "JOSE interactive mode".to_string(),
                },
                ChatMessage {
                    role: MessageRole::System,
                    content: "Enter sends. Newline: Shift+Enter, Alt+Enter, or Ctrl+J. Press Esc or Ctrl+C to exit.".to_string(),
                },
            ],
            chat_scroll: 0,
            auto_follow: true,
        }
    }

    fn push_user_message(&mut self, msg: &str) {
        self.messages.push(ChatMessage {
            role: MessageRole::User,
            content: msg.to_string(),
        });
    }

    fn push_assistant_message(&mut self, msg: &str) {
        self.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: msg.to_string(),
        });
    }
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
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

fn render_chat_lines(state: &ChatState, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let content_width = width.saturating_sub(2).max(1);

    for message in &state.messages {
        let (label, color) = match message.role {
            MessageRole::System => ("System", Color::DarkGray),
            MessageRole::User => ("You", Color::Gray),
            MessageRole::Assistant => ("Jose", Color::LightBlue),
        };

        lines.push(Line::from(vec![
            Span::styled("● ", Style::default().fg(color)),
            Span::styled(label, Style::default().fg(color).add_modifier(Modifier::BOLD)),
        ]));

        for wrapped in wrap_text(&message.content, content_width) {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default().fg(Color::DarkGray)),
                Span::raw(wrapped),
            ]));
        }

        lines.push(Line::from(Span::styled("", Style::default())));
    }

    lines
}

fn chat_max_scroll(state: &ChatState, chat_area: Rect) -> usize {
    let lines = render_chat_lines(state, chat_area.width.saturating_sub(2) as usize);
    let visible = chat_area.height.saturating_sub(2) as usize;
    lines.len().saturating_sub(visible.max(1))
}

fn cursor_to_row_col(text: &str, cursor_pos: usize, width: usize) -> (usize, usize) {
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
    
    // Handle cursor at end of line
    if pos > line_start {
        let line_pos = pos - line_start;
        col = line_pos % width;
        if line_pos > 0 && line_pos % width == 0 && pos < chars.len() {
            row += 1;
            col = 0;
        }
    }
    
    (row, col)
}

fn row_col_to_cursor(text: &str, target_row: usize, target_col: usize, width: usize) -> usize {
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
                // Return end of this line
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
    
    // Target row is at or after last line
    if row == target_row {
        let line_len = chars.len() - line_start;
        return (line_start + target_col.min(line_len)).min(chars.len());
    }
    
    chars.len()
}

fn cursor_blink_on() -> bool {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    (millis / 500).is_multiple_of(2)
}

fn draw_ui(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, state: &ChatState) -> Result<()> {
    terminal.draw(|frame| {
        let size = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(7),
                Constraint::Length(1),
            ])
            .split(size);

        let chat_lines = render_chat_lines(state, chunks[0].width.saturating_sub(2) as usize);
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
            .wrap(Wrap { trim: false })
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
        
        // Auto-scroll to keep cursor visible
        let input_scroll = if cursor_row >= input_visible_lines {
            cursor_row.saturating_sub(input_visible_lines - 1)
        } else {
            0
        };
        let input_start = input_scroll.min(input_total_lines.saturating_sub(input_visible_lines.max(1)));
        let input_end = (input_start + input_visible_lines.max(1)).min(input_total_lines);
        let mut input_slice = wrapped_input[input_start..input_end].to_vec();

        // Insert cursor at actual position
        if cursor_blink_on() && cursor_row >= input_start && cursor_row < input_end {
            let idx = cursor_row - input_start;
            if idx < input_slice.len() {
                let line: Vec<char> = input_slice[idx].chars().collect();
                let col = cursor_col.min(line.len());
                let mut new_line: String = line[..col].iter().collect();
                new_line.push('│');
                new_line.extend(line[col..].iter());
                input_slice[idx] = new_line;
            }
        }

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
            Span::styled("Arrows", Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            Span::raw("=move  "),
            Span::styled("Alt+←/→", Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            Span::raw("=word  "),
            Span::styled("Scroll", Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            Span::raw("=chat  Esc=exit"),
        ]))
        .style(Style::default().fg(Color::DarkGray));

        frame.render_widget(chat, chunks[0]);
        frame.render_stateful_widget(chat_scrollbar, chunks[0], &mut chat_scrollbar_state);
        frame.render_widget(input, chunks[1]);
        frame.render_stateful_widget(input_scrollbar, chunks[1], &mut input_scrollbar_state);
        frame.render_widget(hint, chunks[2]);
    })?;

    Ok(())
}

pub fn run_interactive(model_override: Option<&str>) -> Result<()> {
    let config = Config::load()?;
    let model = model_override
        .map(ToString::to_string)
        .unwrap_or(config.default_model);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    )?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, model);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        PopKeyboardEnhancementFlags,
        DisableMouseCapture,
        DisableBracketedPaste,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    result
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, model: String) -> Result<()> {
    let mut state = ChatState::new(model);

    loop {
        draw_ui(terminal, &state)?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        match event::read()? {
            Event::Paste(text) => {
                let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
                let chars: Vec<char> = state.input.chars().collect();
                let pos = state.cursor_pos.min(chars.len());
                let before: String = chars[..pos].iter().collect();
                let after: String = chars[pos..].iter().collect();
                state.input = format!("{}{}{}", before, normalized, after);
                state.cursor_pos = pos + normalized.chars().count();
            }
            Event::Mouse(mouse) => {
                match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        let area = terminal.size()?;
                        let chunks = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints([
                                Constraint::Min(3),
                                Constraint::Length(7),
                                Constraint::Length(1),
                            ])
                            .split(area.into());
                        let max_scroll = chat_max_scroll(&state, chunks[0]);
                        if state.auto_follow {
                            state.auto_follow = false;
                            state.chat_scroll = max_scroll;
                        }
                        state.chat_scroll = state.chat_scroll.saturating_sub(3);
                    }
                    MouseEventKind::ScrollDown => {
                        let area = terminal.size()?;
                        let chunks = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints([
                                Constraint::Min(3),
                                Constraint::Length(7),
                                Constraint::Length(1),
                            ])
                            .split(area.into());
                        let max_scroll = chat_max_scroll(&state, chunks[0]);
                        if !state.auto_follow {
                            state.chat_scroll = (state.chat_scroll + 3).min(max_scroll);
                            if state.chat_scroll >= max_scroll {
                                state.auto_follow = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Esc => return Ok(()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(())
                }
                KeyCode::PageUp => {
                    let area = terminal.size()?;
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Min(3),
                            Constraint::Length(7),
                            Constraint::Length(1),
                        ])
                        .split(area.into());
                    let max_scroll = chat_max_scroll(&state, chunks[0]);
                    if state.auto_follow {
                        state.auto_follow = false;
                        state.chat_scroll = max_scroll;
                    }
                    state.chat_scroll = state.chat_scroll.saturating_sub(8);
                }
                KeyCode::PageDown => {
                    let area = terminal.size()?;
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Min(3),
                            Constraint::Length(7),
                            Constraint::Length(1),
                        ])
                        .split(area.into());
                    let max_scroll = chat_max_scroll(&state, chunks[0]);
                    if !state.auto_follow {
                        state.chat_scroll = (state.chat_scroll + 8).min(max_scroll);
                        if state.chat_scroll >= max_scroll {
                            state.auto_follow = true;
                        }
                    }
                }
                KeyCode::Left => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        // Jump to previous word
                        let chars: Vec<char> = state.input.chars().collect();
                        let mut pos = state.cursor_pos.min(chars.len());
                        // Skip whitespace
                        while pos > 0 && chars[pos - 1].is_whitespace() {
                            pos -= 1;
                        }
                        // Skip word
                        while pos > 0 && !chars[pos - 1].is_whitespace() {
                            pos -= 1;
                        }
                        state.cursor_pos = pos;
                    } else {
                        state.cursor_pos = state.cursor_pos.saturating_sub(1);
                    }
                }
                KeyCode::Right => {
                    let len = state.input.chars().count();
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        // Jump to next word
                        let chars: Vec<char> = state.input.chars().collect();
                        let mut pos = state.cursor_pos.min(chars.len());
                        // Skip current word
                        while pos < chars.len() && !chars[pos].is_whitespace() {
                            pos += 1;
                        }
                        // Skip whitespace
                        while pos < chars.len() && chars[pos].is_whitespace() {
                            pos += 1;
                        }
                        state.cursor_pos = pos;
                    } else {
                        state.cursor_pos = (state.cursor_pos + 1).min(len);
                    }
                }
                KeyCode::Up => {
                    // Move cursor up one line
                    let area = terminal.size()?;
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Min(3),
                            Constraint::Length(7),
                            Constraint::Length(1),
                        ])
                        .split(area.into());
                    let width = chunks[1].width.saturating_sub(2) as usize;
                    let (row, col) = cursor_to_row_col(&state.input, state.cursor_pos, width.max(1));
                    if row > 0 {
                        state.cursor_pos = row_col_to_cursor(&state.input, row - 1, col, width.max(1));
                    }
                }
                KeyCode::Down => {
                    // Move cursor down one line
                    let area = terminal.size()?;
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Min(3),
                            Constraint::Length(7),
                            Constraint::Length(1),
                        ])
                        .split(area.into());
                    let width = chunks[1].width.saturating_sub(2) as usize;
                    let wrapped = wrap_text(&state.input, width.max(1));
                    let (row, col) = cursor_to_row_col(&state.input, state.cursor_pos, width.max(1));
                    if row + 1 < wrapped.len() {
                        state.cursor_pos = row_col_to_cursor(&state.input, row + 1, col, width.max(1));
                    } else {
                        state.cursor_pos = state.input.chars().count();
                    }
                }
                KeyCode::Home => {
                    state.cursor_pos = 0;
                }
                KeyCode::End => {
                    state.cursor_pos = state.input.chars().count();
                }
                KeyCode::Enter => {
                    if key.modifiers.contains(KeyModifiers::SHIFT)
                        || key.modifiers.contains(KeyModifiers::ALT)
                    {
                        let chars: Vec<char> = state.input.chars().collect();
                        let pos = state.cursor_pos.min(chars.len());
                        let before: String = chars[..pos].iter().collect();
                        let after: String = chars[pos..].iter().collect();
                        state.input = format!("{}\n{}", before, after);
                        state.cursor_pos = pos + 1;
                    } else {
                        send_current_input(terminal, &mut state)?;
                    }
                }
                KeyCode::Backspace => {
                    if state.cursor_pos > 0 {
                        let chars: Vec<char> = state.input.chars().collect();
                        let pos = state.cursor_pos.min(chars.len());
                        let before: String = chars[..pos - 1].iter().collect();
                        let after: String = chars[pos..].iter().collect();
                        state.input = format!("{}{}", before, after);
                        state.cursor_pos = pos - 1;
                    }
                }
                KeyCode::Delete => {
                    let chars: Vec<char> = state.input.chars().collect();
                    let pos = state.cursor_pos.min(chars.len());
                    if pos < chars.len() {
                        let before: String = chars[..pos].iter().collect();
                        let after: String = chars[pos + 1..].iter().collect();
                        state.input = format!("{}{}", before, after);
                    }
                }
                KeyCode::Char(ch) => {
                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                        if ch == 'j' {
                            let chars: Vec<char> = state.input.chars().collect();
                            let pos = state.cursor_pos.min(chars.len());
                            let before: String = chars[..pos].iter().collect();
                            let after: String = chars[pos..].iter().collect();
                            state.input = format!("{}\n{}", before, after);
                            state.cursor_pos = pos + 1;
                        }
                    } else {
                        let chars: Vec<char> = state.input.chars().collect();
                        let pos = state.cursor_pos.min(chars.len());
                        let before: String = chars[..pos].iter().collect();
                        let after: String = chars[pos..].iter().collect();
                        state.input = format!("{}{}{}", before, ch, after);
                        state.cursor_pos = pos + 1;
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

fn send_current_input(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut ChatState,
) -> Result<()> {
    let prompt = state.input.clone();
    if prompt.trim().is_empty() {
        return Ok(());
    }

    state.push_user_message(&prompt);
    state.push_assistant_message("...thinking...");
    state.input.clear();
    state.cursor_pos = 0;

    if state.auto_follow {
        state.chat_scroll = 0;
    }

    draw_ui(terminal, state)?;

    let response = match call_chatgpt_interactive_with_history(
        &prompt,
        &state.model,
        &state.turns,
        Some(&state.session_id),
    ) {
        Ok(resp) if !resp.trim().is_empty() => {
            state.turns.push(("user".to_string(), prompt.clone()));
            state.turns.push(("assistant".to_string(), resp.clone()));
            resp
        }
        Ok(_) => {
            state.turns.push(("user".to_string(), prompt.clone()));
            "(empty response)".to_string()
        }
        Err(err) => format!("Error: {err}"),
    };

    state.messages.pop();
    state.push_assistant_message(&response);
    state.auto_follow = true;
    Ok(())
}
