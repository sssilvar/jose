use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// OAuth configuration (same as Codex CLI)
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const OAUTH_ISSUER: &str = "https://auth.openai.com";
pub const OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const CHATGPT_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";

/// Must use port 1455 - this is the only port registered with OpenAI's OAuth
pub const OAUTH_PORT: u16 = 1455;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub default_model: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_model: "gpt-5-codex".to_string(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        if path.exists() {
            let content = fs::read_to_string(&path)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&path, content)?;
        Ok(())
    }

    fn config_path() -> Result<PathBuf> {
        let home =
            dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?;
        Ok(home.join(".jose").join("config.json"))
    }
}
