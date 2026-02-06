use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use crate::jwt::parse_jwt_claims;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub id_token: String,
    pub access_token: String,
    pub refresh_token: String,
    pub account_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthData {
    pub tokens: Tokens,
    pub last_refresh: String,
}

impl AuthData {
    pub fn load() -> Result<Option<Self>> {
        let path = Self::auth_path()?;
        if path.exists() {
            let content = fs::read_to_string(&path)?;
            Ok(Some(serde_json::from_str(&content)?))
        } else {
            Ok(None)
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::auth_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&path, &content)?;

        // Set file permissions to 600 (owner read/write only)
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&path, perms)?;

        Ok(())
    }

    fn auth_path() -> Result<PathBuf> {
        let home =
            dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?;
        Ok(home.join(".jose").join("auth.json"))
    }

    /// Check if the access token is expired or about to expire
    pub fn needs_refresh(&self) -> bool {
        if let Some(claims) = parse_jwt_claims(&self.tokens.access_token) {
            if let Some(exp) = claims.get("exp").and_then(|v| v.as_i64()) {
                let now = chrono::Utc::now().timestamp();
                // Refresh if token expires within 5 minutes
                return exp <= now + 300;
            }
        }
        true
    }
}

use crate::config::{CLIENT_ID, OAUTH_TOKEN_URL};

pub fn refresh_tokens(refresh_token: &str) -> Result<Tokens> {
    let client = reqwest::blocking::Client::new();

    let payload = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": CLIENT_ID,
        "scope": "openid profile email offline_access",
    });

    let resp = client
        .post(OAUTH_TOKEN_URL)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .context("Failed to send refresh token request")?;

    if !resp.status().is_success() {
        anyhow::bail!("Token refresh failed: {}", resp.status());
    }

    let data: serde_json::Value = resp.json()?;

    let id_token = data["id_token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing id_token"))?
        .to_string();
    let access_token = data["access_token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing access_token"))?
        .to_string();
    let new_refresh_token = data["refresh_token"]
        .as_str()
        .unwrap_or(refresh_token)
        .to_string();

    // Extract account_id from id_token claims
    let account_id = parse_jwt_claims(&id_token)
        .and_then(|claims| {
            claims
                .get("https://api.openai.com/auth")
                .and_then(|auth| auth.get("chatgpt_account_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();

    Ok(Tokens {
        id_token,
        access_token,
        refresh_token: new_refresh_token,
        account_id,
    })
}

/// Get valid tokens, refreshing if necessary
pub fn get_valid_tokens() -> Result<Option<Tokens>> {
    let auth = match AuthData::load()? {
        Some(auth) => auth,
        None => return Ok(None),
    };

    if auth.needs_refresh() {
        let new_tokens = refresh_tokens(&auth.tokens.refresh_token)?;
        let new_auth = AuthData {
            tokens: new_tokens.clone(),
            last_refresh: chrono::Utc::now().to_rfc3339(),
        };
        new_auth.save()?;
        Ok(Some(new_tokens))
    } else {
        Ok(Some(auth.tokens))
    }
}
