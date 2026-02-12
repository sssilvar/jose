use anyhow::{Context, Result};
use std::io::{BufRead, BufReader};

use crate::auth::{get_valid_tokens, Tokens};
use crate::config::CHATGPT_RESPONSES_URL;
use crate::shell::{detect_shell, os_name};

pub fn call_chatgpt(prompt: &str, model: &str) -> Result<String> {
    call_chatgpt_command(prompt, model)
}

pub fn call_chatgpt_command(prompt: &str, model: &str) -> Result<String> {
    let tokens = get_valid_tokens()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run `jose login` first."))?;

    let os = os_name();
    let shell = detect_shell();

    let system_prompt = format!(
        r#"You are an expert shell command generator for {} using {}.
Respond with ONLY the exact shell command. No explanation. No markdown. No backticks.
If there are alternatives, put them on separate lines."#,
        os,
        shell.name()
    );

    let input = serde_json::json!([
        {"role": "user", "content": prompt}
    ]);

    call_with_tokens(model, &system_prompt, input, &tokens, None, false)
}

pub fn call_chatgpt_interactive_with_history(
    prompt: &str,
    model: &str,
    history: &[(String, String)],
    session_id: Option<&str>,
) -> Result<String> {
    let tokens = get_valid_tokens()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run `jose login` first."))?;

    let system_prompt = r#"You are Jose, a helpful technical assistant in an interactive terminal chat.
Answer naturally and directly.
Do not force shell commands unless the user explicitly asks for one.
Use short, practical explanations by default."#;

    let mut input = Vec::new();
    for (role, content) in history {
        input.push(serde_json::json!({
            "role": role,
            "content": content,
        }));
    }
    input.push(serde_json::json!({
        "role": "user",
        "content": prompt,
    }));

    call_with_tokens(model, system_prompt, serde_json::Value::Array(input), &tokens, session_id, false)
}

fn call_with_tokens(
    model: &str,
    instructions: &str,
    input: serde_json::Value,
    tokens: &Tokens,
    session_id: Option<&str>,
    store: bool,
) -> Result<String> {
    let payload = serde_json::json!({
        "model": model,
        "instructions": instructions,
        "input": input,
        "tools": [],
        "tool_choice": "auto",
        "parallel_tool_calls": false,
        "store": store,
        "stream": true,
    });

    let client = reqwest::blocking::Client::new();
    let mut req = client
        .post(CHATGPT_RESPONSES_URL)
        .header("Authorization", format!("Bearer {}", tokens.access_token))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("chatgpt-account-id", &tokens.account_id)
        .header("OpenAI-Beta", "responses=experimental")
        .json(&payload)
        .timeout(std::time::Duration::from_secs(120));

    if let Some(session_id) = session_id {
        req = req.header("session_id", session_id);
    }

    let resp = req.send().context("Failed to send request to ChatGPT")?;

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
