use anyhow::{Context, Result};
use axum::{
    extract::{Query, State},
    response::{Html, Redirect},
    routing::get,
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::distr::{Alphanumeric, SampleString};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

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

#[derive(Clone)]
struct OAuthState {
    pkce: PkceCodes,
    #[allow(dead_code)] // Reserved for OAuth state validation
    state: String,
    tokens: Arc<Mutex<Option<Tokens>>>,
    shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

pub fn build_auth_url(pkce: &PkceCodes, state: &str) -> String {
    let redirect_uri = format!("http://localhost:{}/auth/callback", OAUTH_PORT);

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
    let redirect_uri = format!("http://localhost:{}/auth/callback", OAUTH_PORT);

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

async fn handle_callback(
    Query(params): Query<HashMap<String, String>>,
    State(state): State<OAuthState>,
) -> Result<Redirect, Html<String>> {
    let code = match params.get("code") {
        Some(c) => c,
        None => {
            return Err(Html(
                "<h1>Error</h1><p>Missing authorization code</p>".to_string(),
            ));
        }
    };

    // Exchange code for tokens (blocking call in async context)
    let pkce = state.pkce.clone();
    let code = code.clone();

    let tokens = tokio::task::spawn_blocking(move || exchange_code(&code, &pkce))
        .await
        .map_err(|e| Html(format!("<h1>Error</h1><p>{}</p>", e)))?
        .map_err(|e| Html(format!("<h1>Error</h1><p>{}</p>", e)))?;

    // Store tokens
    *state.tokens.lock().unwrap() = Some(tokens);

    // Signal shutdown
    if let Some(tx) = state.shutdown_tx.lock().unwrap().take() {
        let _ = tx.send(());
    }

    Ok(Redirect::to(&format!(
        "http://localhost:{}/success",
        OAUTH_PORT
    )))
}

async fn handle_success() -> Html<&'static str> {
    Html(
        r#"
        <html>
        <head><title>Login Successful</title></head>
        <body style="font-family: system-ui; max-width: 600px; margin: 80px auto;">
            <h1>✅ Login Successful!</h1>
            <p>You can close this window and return to the terminal.</p>
        </body>
        </html>
        "#,
    )
}

pub fn do_login() -> Result<bool> {
    log::info("Starting OAuth login flow...");
    log::dim(&format!(
        "Note: Make sure port {} is not in use",
        OAUTH_PORT
    ));

    let pkce = PkceCodes::generate();
    let state_token: String = Alphanumeric.sample_string(&mut rand::rng(), 64);

    let auth_url = build_auth_url(&pkce, &state_token);

    log::info("Opening browser for authentication...");
    log::dim(&format!("If browser doesn't open, visit:\n{}", auth_url));

    // Open browser
    if let Err(e) = open::that(&auth_url) {
        log::warn(&format!("Failed to open browser: {}", e));
    }

    // Create tokio runtime for the server
    let rt = tokio::runtime::Runtime::new()?;

    let result = rt.block_on(async {
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let oauth_state = OAuthState {
            pkce,
            state: state_token,
            tokens: Arc::new(Mutex::new(None)),
            shutdown_tx: Arc::new(Mutex::new(Some(shutdown_tx))),
        };

        let tokens_ref = oauth_state.tokens.clone();

        let app = Router::new()
            .route("/auth/callback", get(handle_callback))
            .route("/success", get(handle_success))
            .with_state(oauth_state);

        let addr = format!("127.0.0.1:{}", OAUTH_PORT);
        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                log::error(&format!("Port {} is already in use.", OAUTH_PORT));
                log::info("Make sure ChatMock or another instance is not running.");
                return Err(anyhow::anyhow!("Failed to bind: {}", e));
            }
        };

        log::info("Waiting for authentication callback...");

        // Serve until we get a shutdown signal
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
                // Give time for the success page to be served
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            })
            .await?;

        let tokens = tokens_ref.lock().unwrap().take();
        Ok(tokens)
    })?;

    if let Some(tokens) = result {
        let auth = AuthData {
            tokens,
            last_refresh: chrono::Utc::now().to_rfc3339(),
        };
        auth.save()?;
        log::success("Login successful! Credentials saved.");
        Ok(true)
    } else {
        log::error("Login failed.");
        Ok(false)
    }
}
