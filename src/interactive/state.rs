use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub(crate) enum MessageRole {
    System,
    User,
    Assistant,
}

pub(crate) struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

/// A clickable copy button rendered in the chat area.
pub(crate) struct CopyButton {
    /// Visual line index (in the full chat_lines, before scroll).
    pub line: usize,
    /// Column start of the button text.
    pub col_start: usize,
    /// The code content to copy when clicked.
    pub content: String,
}

pub(crate) struct ChatState {
    pub model: String,
    pub session_id: String,
    pub turns: Vec<(String, String)>,
    pub input: String,
    pub cursor_pos: usize,
    pub messages: Vec<ChatMessage>,
    pub chat_scroll: usize,
    pub auto_follow: bool,
    /// Selection range as (start, end) offsets into flat chat plain text.
    pub selection: Option<(usize, usize)>,
    /// Anchor offset set on mouse-down, used during drag.
    pub drag_anchor: Option<usize>,
    /// Timestamp and position of last left-click for double-click detection.
    pub last_click: Option<(Instant, u16, u16)>,
    /// Copy buttons for code blocks, rebuilt on each render.
    pub copy_buttons: Vec<CopyButton>,
    /// Plain-text mirror of visual chat lines, rebuilt on each render.
    pub plain_lines: Vec<String>,
}

impl ChatState {
    pub fn new(model: String) -> Self {
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
            selection: None,
            drag_anchor: None,
            last_click: None,
            copy_buttons: Vec::new(),
            plain_lines: Vec::new(),
        }
    }

    pub fn push_user_message(&mut self, msg: &str) {
        self.messages.push(ChatMessage {
            role: MessageRole::User,
            content: msg.to_string(),
        });
    }

    pub fn push_assistant_message(&mut self, msg: &str) {
        self.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: msg.to_string(),
        });
    }
}
