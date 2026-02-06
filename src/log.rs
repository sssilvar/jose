//! Cross-platform logging utilities with colored output

use std::io::{self, Write};

/// ANSI color codes
pub mod colors {
    pub const RESET: &str = "\x1b[0m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const RED: &str = "\x1b[31m";
    pub const CYAN: &str = "\x1b[36m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";
}

/// Check if stdout supports colors
fn supports_color() -> bool {
    // Check NO_COLOR environment variable (https://no-color.org/)
    if std::env::var("NO_COLOR").is_ok() {
        return false;
    }
    // Check if stdout is a terminal
    atty::is(atty::Stream::Stdout)
}

/// Format text with color if supported
fn colorize(text: &str, color: &str) -> String {
    if supports_color() {
        format!("{}{}{}", color, text, colors::RESET)
    } else {
        text.to_string()
    }
}

/// Log an info message
pub fn info(message: &str) {
    println!("{} {}", colorize("[*]", colors::CYAN), message);
}

/// Log a success message
pub fn success(message: &str) {
    println!("{} {}", colorize("[+]", colors::GREEN), message);
}

/// Log a warning message
pub fn warn(message: &str) {
    eprintln!("{} {}", colorize("[!]", colors::YELLOW), message);
}

/// Log an error message
pub fn error(message: &str) {
    eprintln!("{} {}", colorize("[-]", colors::RED), message);
}

/// Log a debug/dim message
pub fn dim(message: &str) {
    println!("{}", colorize(message, colors::DIM));
}

/// Print a command (highlighted)
pub fn command(cmd: &str) {
    println!("    {}", colorize(cmd, colors::BOLD));
}

/// Print without newline and flush
#[allow(dead_code)] // Utility function for future use
pub fn print_inline(message: &str) {
    print!("{}", message);
    let _ = io::stdout().flush();
}
