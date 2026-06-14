mod auth;
mod clipboard;
mod config;
mod jwt;
mod log;
mod oauth;
mod prompt;
mod provider;
mod shell;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::auth::AuthData;
use crate::clipboard::copy_to_clipboard;
use crate::config::{Config, ProviderKind, AVAILABLE_MODELS};
use crate::jwt::parse_jwt_claims;
use crate::oauth::do_login;

#[derive(Parser)]
#[command(name = "jose")]
#[command(version)]
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
    /// Show the current model and available models, or set a new one
    Model {
        #[command(subcommand)]
        command: Option<ModelCommands>,
    },
    /// Show the current provider, or switch/configure one
    Provider {
        #[command(subcommand)]
        command: Option<ProviderCommands>,
    },
}

#[derive(Subcommand)]
enum ModelCommands {
    /// Set the default model
    Set {
        /// The model name to set as default
        model: String,
    },
}

#[derive(Subcommand)]
enum ProviderCommands {
    /// Set the active provider
    Set {
        #[command(subcommand)]
        kind: ProviderSet,
    },
}

#[derive(Subcommand)]
enum ProviderSet {
    /// Use the ChatGPT subscription backend (OAuth)
    Chatgpt,
    /// Use an OpenAI-compatible server (ollama, llama.cpp, vLLM, ...)
    #[command(name = "openai-compatible")]
    OpenAiCompatible {
        /// Base URL including the version path, e.g. https://foo.bar/v1
        #[arg(long)]
        base_url: String,
        /// Optional API key (sent as a Bearer token)
        #[arg(long)]
        api_key: Option<String>,
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

fn cmd_model_show() -> Result<()> {
    let config = Config::load()?;
    log::success(&format!("Current model: {}", config.default_model));
    log::info("Available models:");
    for model in AVAILABLE_MODELS {
        if *model == config.default_model {
            log::command(&format!("{} (current)", model));
        } else {
            log::command(model);
        }
    }
    Ok(())
}

fn cmd_model_set(model: &str) -> Result<()> {
    let mut config = Config::load()?;
    // The known-model list only applies to the ChatGPT backend; openai-compatible
    // servers expose arbitrary model names.
    if config.provider == ProviderKind::Chatgpt && !AVAILABLE_MODELS.contains(&model) {
        log::warn(&format!(
            "`{}` is not in the known model list. Setting it anyway.",
            model
        ));
    }
    config.default_model = model.to_string();
    config.save()?;
    log::success(&format!("Default model set to: {}", model));
    Ok(())
}

fn cmd_provider_show() -> Result<()> {
    let config = Config::load()?;
    log::success(&format!("Current provider: {}", config.provider.as_str()));
    if config.provider == ProviderKind::OpenAiCompatible {
        match config.base_url() {
            Some(url) => log::info(&format!("Base URL: {}", url)),
            None => log::warn("No base URL set (use `provider set openai-compatible --base-url`)"),
        }
        log::info(&format!(
            "API key: {}",
            if config.api_key().is_some() { "set" } else { "none" }
        ));
    }
    Ok(())
}

fn cmd_provider_set(set: &ProviderSet) -> Result<()> {
    let mut config = Config::load()?;
    match set {
        ProviderSet::Chatgpt => {
            config.provider = ProviderKind::Chatgpt;
            log::success("Provider set to: chatgpt");
        }
        ProviderSet::OpenAiCompatible { base_url, api_key } => {
            config.provider = ProviderKind::OpenAiCompatible;
            config.base_url = Some(base_url.clone());
            if api_key.is_some() {
                config.api_key = api_key.clone();
            }
            log::success(&format!("Provider set to: openai-compatible ({})", base_url));
        }
    }
    config.save()?;
    Ok(())
}

fn cmd_query(prompt: &str, model: Option<&str>) -> Result<()> {
    let config = Config::load()?;
    let model = model.unwrap_or(&config.default_model);

    log::info(&format!("Querying {}...", model));

    let result = provider::generate(&config, prompt, model)?;

    if result.is_empty() {
        anyhow::bail!("Empty response from provider");
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
        Some(Commands::Model { command }) => match command {
            None => cmd_model_show()?,
            Some(ModelCommands::Set { model }) => cmd_model_set(&model)?,
        },
        Some(Commands::Provider { command }) => match command {
            None => cmd_provider_show()?,
            Some(ProviderCommands::Set { kind }) => cmd_provider_set(&kind)?,
        },
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
