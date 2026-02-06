use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::Value;

/// Parse JWT claims from a token (without verification)
pub fn parse_jwt_claims(token: &str) -> Option<Value> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    let payload = parts[1];
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&decoded).ok()
}
