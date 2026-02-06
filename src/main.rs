mod auth;
mod chatgpt;
mod clipboard;
mod config;
mod jwt;
mod log;
mod oauth;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::auth::AuthData;
use crate::chatgpt::call_chatgpt;
use crate::clipboard::copy_to_clipboard;
use crate::config::Config;
use crate::jwt::parse_jwt_claims;
use crate::oauth::do_login;

#[derive(Parser)]
#[command(name = "jose")]
#[command(about = "CLI tool using ChatGPT subscription for shell commands", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Prompt for command generation (when no subcommand is used)
    #[arg(trailing_var_arg = true)]
    prompt: Vec<String>,

    /// Model to use (e.g., gpt-5, gpt-5-codex)
    #[arg(short, long)]
    model: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Authenticate with ChatGPT
    Login,
    /// Show authentication status
    Info,
    /// Set the default model
    SetModel {
        /// The model name to set as default
        model: String,
    },
}

fn cmd_info() -> Result<()> {
    match AuthData::load()? {
        Some(auth) => {
            if let Some(claims) = parse_jwt_claims(&auth.tokens.access_token) {
                if let Some(exp) = claims.get("exp").and_then(|v| v.as_i64()) {
                    let expiry = chrono::DateTime::from_timestamp(exp, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    log::success(&format!("Authenticated. Token expires: {}", expiry));
                } else {
                    log::success("Authenticated.");
                }
            } else {
                log::warn("Auth file exists but token could not be parsed.");
            }
        }
        None => {
            log::error("Not authenticated. Run `jose login`");
        }
    }
    Ok(())
}

fn cmd_set_model(model: &str) -> Result<()> {
    let mut config = Config::load()?;
    config.default_model = model.to_string();
    config.save()?;
    log::success(&format!("Default model set to: {}", model));
    Ok(())
}

fn cmd_query(prompt: &str, model: Option<&str>) -> Result<()> {
    let config = Config::load()?;
    let model = model.unwrap_or(&config.default_model);

    log::info(&format!("Querying {}...", model));

    let result = call_chatgpt(prompt, model)?;

    if result.is_empty() {
        anyhow::bail!("Empty response from ChatGPT");
    }

    // Get first line as main command
    let lines: Vec<&str> = result.lines().collect();
    let command = lines.first().unwrap_or(&"");

    // Copy to clipboard
    if let Err(e) = copy_to_clipboard(command) {
        log::warn(&format!("Failed to copy to clipboard: {}", e));
    } else {
        log::success("Command copied to clipboard:");
    }

    log::command(command);

    // Show alternatives if any
    if lines.len() > 1 {
        let alternatives: Vec<&str> = lines[1..]
            .iter()
            .filter(|l| !l.trim().is_empty())
            .copied()
            .collect();

        if !alternatives.is_empty() {
            log::info("Alternatives:");
            for alt in alternatives {
                log::command(alt);
            }
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Login) => {
            if do_login()? {
                std::process::exit(0);
            } else {
                std::process::exit(1);
            }
        }
        Some(Commands::Info) => {
            cmd_info()?;
        }
        Some(Commands::SetModel { model }) => {
            cmd_set_model(&model)?;
        }
        None => {
            if cli.prompt.is_empty() {
                log::error("Please provide a prompt or use a subcommand.");
                log::info("Run `jose --help` for usage.");
                std::process::exit(1);
            }

            let prompt = cli.prompt.join(" ");
            cmd_query(&prompt, cli.model.as_deref())?;
        }
    }

    Ok(())
}
