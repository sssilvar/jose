use anyhow::{Context, Result};
use std::io::{BufRead, BufReader};

use crate::auth::{get_valid_tokens, Tokens};
use crate::config::CHATGPT_RESPONSES_URL;
use crate::shell::{detect_shell, os_name};

pub fn call_chatgpt(prompt: &str, model: &str) -> Result<String> {
    let tokens = get_valid_tokens()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run `jose login` first."))?;

    call_with_tokens(prompt, model, &tokens)
}

fn call_with_tokens(prompt: &str, model: &str, tokens: &Tokens) -> Result<String> {
    let os = os_name();
    let shell = detect_shell();

    let system_prompt = format!(
        r#"You are an expert shell command generator for {} using {}.
Respond with ONLY the exact shell command. No explanation. No markdown. No backticks.
If there are alternatives, put them on separate lines."#,
        os,
        shell.name()
    );

    let payload = serde_json::json!({
        "model": model,
        "instructions": system_prompt,
        "input": [
            {"role": "user", "content": prompt}
        ],
        "tools": [],
        "tool_choice": "auto",
        "parallel_tool_calls": false,
        "store": false,
        "stream": true,
    });

    let client = reqwest::blocking::Client::new();

    let resp = client
        .post(CHATGPT_RESPONSES_URL)
        .header("Authorization", format!("Bearer {}", tokens.access_token))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("chatgpt-account-id", &tokens.account_id)
        .header("OpenAI-Beta", "responses=experimental")
        .json(&payload)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .context("Failed to send request to ChatGPT")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        anyhow::bail!("API error: {} - {}", status, body);
    }

    // Parse SSE stream
    let mut full_response = String::new();
    let reader = BufReader::new(resp);

    for line in reader.lines() {
        let line = line?;
        if line.is_empty() {
            continue;
        }

        if let Some(data) = line.strip_prefix("data: ") {
            if data == "[DONE]" {
                break;
            }

            if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                // Handle various event types
                if event.get("type") == Some(&serde_json::json!("response.output_text.delta")) {
                    if let Some(delta) = event.get("delta").and_then(|d| d.as_str()) {
                        full_response.push_str(delta);
                    }
                } else if let Some(delta) = event.get("delta") {
                    if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                        full_response.push_str(text);
                    } else if let Some(text) = delta.as_str() {
                        full_response.push_str(text);
                    }
                }
            }
        }
    }

    Ok(full_response.trim().to_string())
}
