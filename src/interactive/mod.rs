mod input;
mod render;
mod selection;
mod state;

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::chatgpt::call_chatgpt_interactive_with_history;
use crate::clipboard::copy_to_clipboard;
use crate::config::Config;

use input::{
    backspace, compute_layout, delete, delete_prev_word, insert_char, insert_newline,
    move_down, move_left, move_right, move_up,
};
use render::{chat_max_scroll, draw_ui};
use selection::{extract_selection, handle_mouse};
use state::ChatState;

// ── Public entry point ────────────────────────────────────────────────

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
        EnableMouseCapture
    )?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, model);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        DisableBracketedPaste,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    result
}

// ── Ctrl+C helper ─────────────────────────────────────────────────────

/// Returns true if the event loop should `continue`, false if it should exit.
fn handle_copy_or_exit(state: &mut ChatState) -> Result<bool> {
    // 1. Selection active → copy to clipboard
    if let Some((a, b)) = state.selection {
        if a != b {
            let text = extract_selection(&state.plain_lines, (a, b));
            let _ = copy_to_clipboard(&text);
            state.selection = None;
            state.drag_anchor = None;
            return Ok(true); // continue loop
        }
    }
    // 2. Input non-empty → clear input
    if !state.input.is_empty() {
        state.input.clear();
        state.cursor_pos = 0;
        return Ok(true);
    }
    // 3. Exit
    Ok(false)
}

/// Check if a key event is Ctrl+C / Cmd+C (any terminal encoding variant).
fn is_ctrl_c(code: KeyCode, modifiers: KeyModifiers) -> bool {
    matches!(
        (code, modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::META | KeyModifiers::SUPER)),
        (KeyCode::Char('c'), true) | (KeyCode::Char('\x03'), _)
    )
}

// ── Main event loop ───────────────────────────────────────────────────

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, model: String) -> Result<()> {
    let mut state = ChatState::new(model);

    loop {
        draw_ui(terminal, &mut state)?;

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
                let area = terminal.size()?;
                let chunks = compute_layout(area.into());
                let chat_area = chunks[0];
                let max_scroll = chat_max_scroll(&state, chat_area);
                let scroll = if state.auto_follow {
                    max_scroll
                } else {
                    state.chat_scroll.min(max_scroll)
                } as u16;

                handle_mouse(
                    &mut state,
                    mouse.kind,
                    mouse.row,
                    mouse.column,
                    chat_area,
                    scroll,
                    max_scroll,
                );
            }

            Event::Key(key) if key.kind == KeyEventKind::Press => {
                // ── Ctrl+C: copy / clear / exit ──────────────────
                if is_ctrl_c(key.code, key.modifiers) {
                    if !handle_copy_or_exit(&mut state)? {
                        return Ok(());
                    }
                    continue;
                }

                // Clear chat selection on any other key
                state.selection = None;
                state.drag_anchor = None;

                let area = terminal.size()?;
                let chunks = compute_layout(area.into());
                let input_width = chunks[1].width.saturating_sub(2) as usize;

                match key.code {
                    KeyCode::Esc => {
                        // Option/Alt chord: ESC-prefixed sequence
                        if event::poll(Duration::from_millis(20))? {
                            if let Event::Key(next) = event::read()? {
                                if next.kind == KeyEventKind::Press
                                    && matches!(next.code, KeyCode::Backspace | KeyCode::Delete)
                                {
                                    delete_prev_word(&mut state.input, &mut state.cursor_pos);
                                    continue;
                                }
                            }
                        }
                        return Ok(());
                    }
                    KeyCode::PageUp => {
                        let max_scroll = chat_max_scroll(&state, chunks[0]);
                        if state.auto_follow {
                            state.auto_follow = false;
                            state.chat_scroll = max_scroll;
                        }
                        state.chat_scroll = state.chat_scroll.saturating_sub(8);
                    }
                    KeyCode::PageDown => {
                        let max_scroll = chat_max_scroll(&state, chunks[0]);
                        if !state.auto_follow {
                            state.chat_scroll = (state.chat_scroll + 8).min(max_scroll);
                            if state.chat_scroll >= max_scroll {
                                state.auto_follow = true;
                            }
                        }
                    }
                    KeyCode::Left => move_left(&mut state, key.modifiers),
                    KeyCode::Right => move_right(&mut state, key.modifiers),
                    KeyCode::Up => move_up(&mut state, input_width),
                    KeyCode::Down => move_down(&mut state, input_width),
                    KeyCode::Home => state.cursor_pos = 0,
                    KeyCode::End => state.cursor_pos = state.input.chars().count(),
                    KeyCode::Enter => {
                        if key.modifiers.contains(KeyModifiers::SHIFT)
                            || key.modifiers.contains(KeyModifiers::ALT)
                        {
                            insert_newline(&mut state);
                        } else {
                            send_current_input(terminal, &mut state)?;
                        }
                    }
                    KeyCode::Backspace => backspace(&mut state, key.modifiers),
                    KeyCode::Delete => delete(&mut state, key.modifiers),
                    KeyCode::Char(ch) => {
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            || key.modifiers.contains(KeyModifiers::META)
                            || key.modifiers.contains(KeyModifiers::SUPER)
                        {
                            match ch {
                                'j' => insert_newline(&mut state),
                                'w' => delete_prev_word(&mut state.input, &mut state.cursor_pos),
                                'c' => {} // handled above
                                _ => {}
                            }
                        } else if (key.modifiers.intersects(KeyModifiers::ALT) && ch == 'w')
                            || ch == '\u{17}'
                        {
                            delete_prev_word(&mut state.input, &mut state.cursor_pos);
                        } else if ch == '\x03' {
                            // Raw ETX — handled above
                        } else if !ch.is_control() {
                            insert_char(&mut state, ch);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

// ── Send message ──────────────────────────────────────────────────────

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
