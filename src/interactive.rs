use std::io;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
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
            messages: vec![
                ChatMessage {
                    role: MessageRole::System,
                    content: "JOSE interactive mode".to_string(),
                },
                ChatMessage {
                    role: MessageRole::System,
                    content: "Enter prompt and press Enter. Press Esc or Ctrl+C to exit. Use Shift+Enter for newline.".to_string(),
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
            MessageRole::User => ("You", Color::Cyan),
            MessageRole::Assistant => ("Jose", Color::Magenta),
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
        let max_scroll = {
            let visible = chunks[0].height.saturating_sub(2) as usize;
            chat_lines.len().saturating_sub(visible.max(1))
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
                    .border_style(Style::default().fg(Color::Blue)),
            )
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));

        let input_inner_width = chunks[1].width.saturating_sub(2) as usize;
        let wrapped_input = wrap_text(&state.input, input_inner_width.max(1));
        let input_visible_lines = chunks[1].height.saturating_sub(2) as usize;
        let input_start = wrapped_input.len().saturating_sub(input_visible_lines.max(1));
        let input_text = wrapped_input[input_start..].join("\n");

        let input = Paragraph::new(input_text)
            .block(
                Block::default()
                    .title(" Input ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Green)),
            )
            .wrap(Wrap { trim: false });

        let hint = Paragraph::new(Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw("=send  "),
            Span::styled("Shift+Enter", Style::default().fg(Color::Yellow)),
            Span::raw("=newline  "),
            Span::styled("PgUp/PgDn", Style::default().fg(Color::Yellow)),
            Span::raw("=scroll  Esc/Ctrl+C=exit"),
        ]))
        .style(Style::default().fg(Color::DarkGray));

        frame.render_widget(chat, chunks[0]);
        frame.render_widget(input, chunks[1]);
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
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, model);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), DisableBracketedPaste, LeaveAlternateScreen)?;
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
                state.input.push_str(&normalized);
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
                    if state.auto_follow {
                        continue;
                    }
                    state.chat_scroll = (state.chat_scroll + 8).min(max_scroll);
                    if state.chat_scroll >= max_scroll {
                        state.auto_follow = true;
                    }
                }
                KeyCode::Up => {
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
                    state.chat_scroll = state.chat_scroll.saturating_sub(1);
                }
                KeyCode::Down => {
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
                        continue;
                    }
                    state.chat_scroll = (state.chat_scroll + 1).min(max_scroll);
                    if state.chat_scroll >= max_scroll {
                        state.auto_follow = true;
                    }
                }
                KeyCode::Enter => {
                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                        state.input.push('\n');
                        continue;
                    }

                    let prompt = state.input.clone();
                    if prompt.trim().is_empty() {
                        continue;
                    }

                    state.push_user_message(&prompt);
                    state.push_assistant_message("...thinking...");
                    state.input.clear();

                    if state.auto_follow {
                        state.chat_scroll = 0;
                    }

                    draw_ui(terminal, &state)?;

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
                }
                KeyCode::Backspace => {
                    state.input.pop();
                }
                KeyCode::Char(ch) => {
                    state.input.push(ch);
                }
                _ => {}
            },
            _ => {}
        }
    }
}
