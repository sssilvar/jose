use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::distr::{Alphanumeric, SampleString};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use crate::auth::{AuthData, Tokens};
use crate::config::{CLIENT_ID, OAUTH_ISSUER, OAUTH_PORT, OAUTH_TOKEN_URL};
use crate::jwt::parse_jwt_claims;
use crate::log;

#[derive(Debug, Clone)]
pub struct PkceCodes {
    pub code_verifier: String,
    pub code_challenge: String,
}

impl PkceCodes {
    pub fn generate() -> Self {
        let code_verifier: String = Alphanumeric.sample_string(&mut rand::rng(), 128);
        let digest = Sha256::digest(code_verifier.as_bytes());
        let code_challenge = URL_SAFE_NO_PAD.encode(digest);

        Self {
            code_verifier,
            code_challenge,
        }
    }
}

fn redirect_uri() -> String {
    format!("http://localhost:{}/auth/callback", OAUTH_PORT)
}

pub fn build_auth_url(pkce: &PkceCodes, state: &str) -> String {
    let redirect_uri = redirect_uri();

    let params = [
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", &redirect_uri),
        ("scope", "openid profile email offline_access"),
        ("code_challenge", &pkce.code_challenge),
        ("code_challenge_method", "S256"),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("state", state),
    ];

    let query = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&");

    format!("{}/oauth/authorize?{}", OAUTH_ISSUER, query)
}

fn exchange_code(code: &str, pkce: &PkceCodes) -> Result<Tokens> {
    let redirect_uri = redirect_uri();
    let client = reqwest::blocking::Client::new();

    let body = format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
        urlencoding::encode(code),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(CLIENT_ID),
        urlencoding::encode(&pkce.code_verifier)
    );

    let resp = client
        .post(OAUTH_TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .context("Failed to exchange code")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        anyhow::bail!("Token exchange failed: {} - {}", status, body);
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
    let refresh_token = data["refresh_token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing refresh_token"))?
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
        refresh_token,
        account_id,
    })
}

/// Parse the query string of the request line `GET /path?query HTTP/1.1`.
fn parse_request_query(request_line: &str) -> HashMap<String, String> {
    request_line
        .split_whitespace()
        .nth(1)
        .and_then(|path| path.split_once('?'))
        .map(|(_, query)| {
            query
                .split('&')
                .filter_map(|pair| pair.split_once('='))
                .map(|(k, v)| {
                    (
                        k.to_string(),
                        urlencoding::decode(v).map(|s| s.into_owned()).unwrap_or_default(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

const SUCCESS_HTML: &str = r#"<html>
<head><title>Login Successful</title></head>
<body style="font-family: system-ui; max-width: 600px; margin: 80px auto;">
    <h1>Login Successful</h1>
    <p>You can close this window and return to the terminal.</p>
</body>
</html>"#;

fn http_response(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Block on a one-shot HTTP server until the OAuth callback delivers a code.
fn wait_for_callback(listener: &TcpListener, pkce: &PkceCodes, state: &str) -> Result<Tokens> {
    for stream in listener.incoming() {
        let mut stream = stream?;

        let mut request_line = String::new();
        BufReader::new(&stream).read_line(&mut request_line)?;

        // Ignore anything that isn't the OAuth callback (e.g. favicon).
        if !request_line.contains("/auth/callback") {
            let _ = stream.write_all(http_response("404 Not Found", "Not found").as_bytes());
            continue;
        }

        let params = parse_request_query(&request_line);

        if params.get("state").map(String::as_str) != Some(state) {
            let _ = stream.write_all(
                http_response("400 Bad Request", "<h1>Error</h1><p>State mismatch</p>").as_bytes(),
            );
            anyhow::bail!("OAuth state mismatch - possible CSRF, aborting.");
        }

        let code = params
            .get("code")
            .ok_or_else(|| anyhow::anyhow!("Missing authorization code in callback"))?;

        let tokens = exchange_code(code, pkce)?;
        let _ = stream.write_all(http_response("200 OK", SUCCESS_HTML).as_bytes());
        let _ = stream.flush();
        return Ok(tokens);
    }

    anyhow::bail!("Listener closed before receiving callback")
}

pub fn do_login() -> Result<bool> {
    log::info("Starting OAuth login flow...");
    log::dim(&format!("Note: Make sure port {} is not in use", OAUTH_PORT));

    let pkce = PkceCodes::generate();
    let state_token: String = Alphanumeric.sample_string(&mut rand::rng(), 64);

    let addr = format!("127.0.0.1:{}", OAUTH_PORT);
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            log::error(&format!("Port {} is already in use.", OAUTH_PORT));
            log::info("Make sure ChatMock or another instance is not running.");
            return Err(anyhow::anyhow!("Failed to bind: {}", e));
        }
    };

    let auth_url = build_auth_url(&pkce, &state_token);

    log::info("Opening browser for authentication...");
    log::dim(&format!("If browser doesn't open, visit:\n{}", auth_url));

    if let Err(e) = open::that(&auth_url) {
        log::warn(&format!("Failed to open browser: {}", e));
    }

    log::info("Waiting for authentication callback...");

    let tokens = wait_for_callback(&listener, &pkce, &state_token)?;

    let auth = AuthData {
        tokens,
        last_refresh: chrono::Utc::now().to_rfc3339(),
    };
    auth.save()?;
    log::success("Login successful! Credentials saved.");
    Ok(true)
}
