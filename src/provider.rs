//! Command-generation backends behind a single entrypoint.

use anyhow::{Context, Result};
use std::io::{BufRead, BufReader};
use std::time::Duration;

use crate::auth::get_valid_tokens;
use crate::config::{Config, ProviderKind, CHATGPT_RESPONSES_URL};
use crate::prompt::build_system_prompt;

/// Generate command suggestions for `prompt` using the configured provider.
pub fn generate(config: &Config, prompt: &str, model: &str) -> Result<String> {
    let system_prompt = build_system_prompt();
    match config.provider {
        ProviderKind::Chatgpt => call_chatgpt(prompt, model, &system_prompt),
        ProviderKind::OpenAiCompatible => call_openai_compatible(config, prompt, model, &system_prompt),
    }
}

/// ChatGPT subscription backend: OAuth bearer + streaming Responses API.
fn call_chatgpt(prompt: &str, model: &str, system_prompt: &str) -> Result<String> {
    let tokens = get_valid_tokens()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run `jose login` first."))?;

    let payload = serde_json::json!({
        "model": model,
        "instructions": system_prompt,
        "input": [{"role": "user", "content": prompt}],
        "tools": [],
        "tool_choice": "auto",
        "parallel_tool_calls": false,
        "store": false,
        "stream": true,
    });

    let resp = reqwest::blocking::Client::new()
        .post(CHATGPT_RESPONSES_URL)
        .header("Authorization", format!("Bearer {}", tokens.access_token))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("chatgpt-account-id", &tokens.account_id)
        .header("OpenAI-Beta", "responses=experimental")
        .json(&payload)
        .timeout(Duration::from_secs(120))
        .send()
        .context("Failed to send request to ChatGPT")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        anyhow::bail!("API error: {} - {}", status, body);
    }

    // Parse SSE stream
    let mut out = String::new();
    for line in BufReader::new(resp).lines() {
        let line = line?;
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        if data == "[DONE]" {
            break;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        if event.get("type") == Some(&serde_json::json!("response.output_text.delta")) {
            if let Some(delta) = event.get("delta").and_then(|d| d.as_str()) {
                out.push_str(delta);
            }
        } else if let Some(delta) = event.get("delta") {
            if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                out.push_str(text);
            } else if let Some(text) = delta.as_str() {
                out.push_str(text);
            }
        }
    }

    Ok(out.trim().to_string())
}

/// OpenAI-compatible backend: `{base_url}/chat/completions`, non-streaming.
fn call_openai_compatible(
    config: &Config,
    prompt: &str,
    model: &str,
    system_prompt: &str,
) -> Result<String> {
    let base_url = config.base_url().ok_or_else(|| {
        anyhow::anyhow!(
            "No base URL set. Run `jose provider set openai-compatible --base-url <url>` \
             or set JOSE_BASE_URL."
        )
    })?;
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    let payload = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": prompt},
        ],
        "stream": false,
    });

    let mut req = reqwest::blocking::Client::new()
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&payload)
        .timeout(Duration::from_secs(120));

    if let Some(key) = config.api_key() {
        req = req.header("Authorization", format!("Bearer {}", key));
    }

    let resp = req
        .send()
        .with_context(|| format!("Failed to send request to {}", url))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        anyhow::bail!("API error: {} - {}", status, body);
    }

    let data: serde_json::Value = resp.json().context("Invalid JSON response")?;
    let content = data["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Unexpected response shape: missing choices[0].message.content"))?;

    Ok(content.trim().to_string())
}
