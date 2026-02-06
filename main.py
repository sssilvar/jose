#!/usr/bin/env python3
"""
jose_gpt.py - CLI tool using ChatGPT subscription via Codex OAuth flow.

This uses the same authentication mechanism as ChatMock to call the ChatGPT API directly.
You need to run `python jose_gpt.py login` first to authenticate with your ChatGPT account.
"""

import sys
import argparse
import base64
import datetime
import hashlib
import http.server
import json
import os
import secrets
import ssl
import urllib.parse
import urllib.request
import webbrowser
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, Optional

import certifi
import requests
import pyperclip

# OAuth configuration (same as Codex CLI)
CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann"
OAUTH_ISSUER = "https://auth.openai.com"
OAUTH_TOKEN_URL = f"{OAUTH_ISSUER}/oauth/token"
CHATGPT_RESPONSES_URL = "https://chatgpt.com/backend-api/codex/responses"

# Local config
HOME_DIR = Path.home() / ".jose-gpt"
AUTH_FILE = HOME_DIR / "auth.json"
CONFIG_FILE = Path(__file__).parent / "config_gpt.json"

# Must use port 1455 - this is the only port registered with OpenAI's OAuth
REQUIRED_PORT = 1455
SSL_CONTEXT = ssl.create_default_context(cafile=certifi.where())


@dataclass
class PkceCodes:
    code_verifier: str
    code_challenge: str


def generate_pkce() -> PkceCodes:
    code_verifier = secrets.token_hex(64)
    digest = hashlib.sha256(code_verifier.encode()).digest()
    code_challenge = base64.urlsafe_b64encode(digest).rstrip(b"=").decode()
    return PkceCodes(code_verifier=code_verifier, code_challenge=code_challenge)


def parse_jwt_claims(token: str) -> Optional[Dict[str, Any]]:
    if not token or token.count(".") != 2:
        return None
    try:
        _, payload, _ = token.split(".")
        padded = payload + "=" * (-len(payload) % 4)
        data = base64.urlsafe_b64decode(padded.encode())
        return json.loads(data.decode())
    except Exception:
        return None


def load_auth() -> Optional[Dict[str, Any]]:
    if AUTH_FILE.exists():
        try:
            with open(AUTH_FILE, "r") as f:
                return json.load(f)
        except Exception:
            return None
    return None


def save_auth(auth: Dict[str, Any]) -> bool:
    try:
        HOME_DIR.mkdir(parents=True, exist_ok=True)
        with open(AUTH_FILE, "w") as f:
            os.chmod(AUTH_FILE, 0o600)
            json.dump(auth, f, indent=2)
        return True
    except Exception as e:
        print(f"Error saving auth: {e}", file=sys.stderr)
        return False


def load_config() -> Dict[str, Any]:
    if CONFIG_FILE.exists():
        with open(CONFIG_FILE, "r") as f:
            return json.load(f)
    return {"default_model": "gpt-5-codex"}


def save_config(config: Dict[str, Any]):
    with open(CONFIG_FILE, "w") as f:
        json.dump(config, f, indent=2)


def refresh_tokens(refresh_token: str) -> Optional[Dict[str, Any]]:
    payload = {
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": CLIENT_ID,
        "scope": "openid profile email offline_access",
    }
    try:
        resp = requests.post(OAUTH_TOKEN_URL, json=payload, timeout=30)
        if resp.status_code >= 400:
            print(f"Token refresh failed: {resp.status_code}", file=sys.stderr)
            return None
        data = resp.json()
        id_token = data.get("id_token")
        access_token = data.get("access_token")
        new_refresh_token = data.get("refresh_token", refresh_token)
        
        # Extract account_id from id_token claims
        claims = parse_jwt_claims(id_token) or {}
        auth_claims = claims.get("https://api.openai.com/auth", {})
        account_id = auth_claims.get("chatgpt_account_id", "")
        
        return {
            "id_token": id_token,
            "access_token": access_token,
            "refresh_token": new_refresh_token,
            "account_id": account_id,
        }
    except Exception as e:
        print(f"Token refresh error: {e}", file=sys.stderr)
        return None


def get_valid_tokens() -> Optional[Dict[str, str]]:
    auth = load_auth()
    if not auth:
        return None
    
    tokens = auth.get("tokens", {})
    access_token = tokens.get("access_token")
    refresh_token = tokens.get("refresh_token")
    account_id = tokens.get("account_id")
    last_refresh = auth.get("last_refresh")
    
    # Check if token needs refresh
    should_refresh = False
    if access_token:
        claims = parse_jwt_claims(access_token) or {}
        exp = claims.get("exp")
        now = datetime.datetime.now(datetime.timezone.utc)
        if isinstance(exp, (int, float)):
            try:
                expiry = datetime.datetime.fromtimestamp(float(exp), datetime.timezone.utc)
                should_refresh = expiry <= now + datetime.timedelta(minutes=5)
            except Exception:
                should_refresh = True
    else:
        should_refresh = True
    
    if should_refresh and refresh_token:
        new_tokens = refresh_tokens(refresh_token)
        if new_tokens:
            auth["tokens"] = new_tokens
            auth["last_refresh"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
            save_auth(auth)
            return new_tokens
        return None
    
    return {
        "access_token": access_token,
        "account_id": account_id,
    }


class OAuthHandler(http.server.BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        pass  # Suppress logs
    
    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)
        
        # Handle success page
        if parsed.path == "/success":
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.end_headers()
            self.wfile.write(b"""
            <html><body style="font-family: system-ui; max-width: 600px; margin: 80px auto;">
            <h1>Login Successful!</h1>
            <p>You can close this window and return to the terminal.</p>
            </body></html>
            """)
            return
        
        if parsed.path == "/auth/callback":
            query = urllib.parse.parse_qs(parsed.query)
            code = query.get("code", [None])[0]
            
            if not code:
                self.send_response(400)
                self.send_header("Content-Type", "text/plain")
                self.end_headers()
                self.wfile.write(b"Missing auth code")
                self.server.shutdown_flag = True
                return
            
            try:
                tokens = self.server.exchange_code(code)
                self.server.tokens = tokens
                self.server.success = True
                
                # Redirect to success page
                self.send_response(302)
                self.send_header("Location", f"http://localhost:{REQUIRED_PORT}/success")
                self.end_headers()
            except Exception as e:
                self.send_response(500)
                self.send_header("Content-Type", "text/plain")
                self.end_headers()
                self.wfile.write(f"Error: {e}".encode())
                print(f"Token exchange error: {e}", file=sys.stderr)
            
            self.server.shutdown_flag = True
        else:
            self.send_response(404)
            self.end_headers()


class OAuthServer(http.server.HTTPServer):
    def __init__(self, port: int):
        # Use 127.0.0.1 explicitly as OpenAI expects
        super().__init__(("127.0.0.1", port), OAuthHandler)
        self.pkce = generate_pkce()
        self.state = secrets.token_hex(32)
        self.redirect_uri = f"http://localhost:{port}/auth/callback"
        self.tokens = None
        self.success = False
        self.shutdown_flag = False
    
    def auth_url(self) -> str:
        params = {
            "response_type": "code",
            "client_id": CLIENT_ID,
            "redirect_uri": self.redirect_uri,
            "scope": "openid profile email offline_access",
            "code_challenge": self.pkce.code_challenge,
            "code_challenge_method": "S256",
            "id_token_add_organizations": "true",
            "codex_cli_simplified_flow": "true",
            "state": self.state,
        }
        return f"{OAUTH_ISSUER}/oauth/authorize?" + urllib.parse.urlencode(params)
    
    def exchange_code(self, code: str) -> Dict[str, Any]:
        data = urllib.parse.urlencode({
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": self.redirect_uri,
            "client_id": CLIENT_ID,
            "code_verifier": self.pkce.code_verifier,
        }).encode()
        
        req = urllib.request.Request(
            OAUTH_TOKEN_URL,
            data=data,
            method="POST",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
        )
        
        with urllib.request.urlopen(req, context=SSL_CONTEXT) as resp:
            payload = json.loads(resp.read().decode())
        
        id_token = payload.get("id_token", "")
        access_token = payload.get("access_token", "")
        refresh_token = payload.get("refresh_token", "")
        
        claims = parse_jwt_claims(id_token) or {}
        auth_claims = claims.get("https://api.openai.com/auth", {})
        account_id = auth_claims.get("chatgpt_account_id", "")
        
        return {
            "id_token": id_token,
            "access_token": access_token,
            "refresh_token": refresh_token,
            "account_id": account_id,
        }


def do_login():
    print("Starting OAuth login flow...")
    print(f"Note: Make sure port {REQUIRED_PORT} is not in use (e.g., by ChatMock)")
    
    try:
        server = OAuthServer(REQUIRED_PORT)
    except OSError as e:
        if e.errno == 48:  # Address already in use
            print(f"Error: Port {REQUIRED_PORT} is already in use.", file=sys.stderr)
            print("Make sure ChatMock or another instance is not running.", file=sys.stderr)
            return False
        raise
    
    auth_url = server.auth_url()
    
    print(f"Opening browser for authentication...")
    print(f"If browser doesn't open, visit:\n{auth_url}")
    webbrowser.open(auth_url)
    
    print("\nWaiting for authentication callback...")
    print("(If the browser can't reach this machine, paste the redirect URL here)")
    
    import threading
    
    def stdin_worker():
        try:
            line = sys.stdin.readline().strip()
            if line and "code=" in line:
                parsed = urllib.parse.urlparse(line)
                params = urllib.parse.parse_qs(parsed.query)
                code = params.get("code", [None])[0]
                if code:
                    print("Processing pasted URL...")
                    try:
                        tokens = server.exchange_code(code)
                        server.tokens = tokens
                        server.success = True
                    except Exception as e:
                        print(f"Error: {e}", file=sys.stderr)
                    server.shutdown_flag = True
        except Exception:
            pass
    
    threading.Thread(target=stdin_worker, daemon=True).start()
    
    while not server.shutdown_flag:
        server.handle_request()
    
    if server.tokens and server.success:
        auth = {
            "tokens": server.tokens,
            "last_refresh": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        }
        if save_auth(auth):
            print("\nLogin successful! Credentials saved.")
            return True
    
    print("\nLogin failed.", file=sys.stderr)
    return False


def call_chatgpt(prompt: str, model: str = "gpt-5-codex") -> Optional[str]:
    tokens = get_valid_tokens()
    if not tokens:
        print("Not authenticated. Run `python jose_gpt.py login` first.", file=sys.stderr)
        return None
    
    access_token = tokens.get("access_token")
    account_id = tokens.get("account_id")
    
    if not access_token or not account_id:
        print("Invalid tokens. Try logging in again.", file=sys.stderr)
        return None
    
    sys_platform = sys.platform
    os_name_map = {
        "linux": "Linux",
        "win32": "Windows",
        "darwin": "macOS",
        "cygwin": "Windows/Cygwin",
    }
    os_name = os_name_map.get(sys_platform, sys_platform)
    
    system_prompt = f"""
You are an expert shell command generator for {os_name}.
Respond with ONLY the exact shell command. No explanation. No markdown. No backticks.
If there are alternatives, put them on separate lines.
"""
    
    payload = {
        "model": model,
        "instructions": system_prompt.strip(),
        "input": [
            {"role": "user", "content": prompt}
        ],
        "tools": [],
        "tool_choice": "auto",
        "parallel_tool_calls": False,
        "store": False,
        "stream": True,
    }
    
    headers = {
        "Authorization": f"Bearer {access_token}",
        "Content-Type": "application/json",
        "Accept": "text/event-stream",
        "chatgpt-account-id": account_id,
        "OpenAI-Beta": "responses=experimental",
    }
    
    try:
        resp = requests.post(
            CHATGPT_RESPONSES_URL,
            headers=headers,
            json=payload,
            stream=True,
            timeout=120,
        )
        
        if resp.status_code >= 400:
            print(f"API error: {resp.status_code} - {resp.text}", file=sys.stderr)
            return None
        
        # Parse SSE stream
        full_response = ""
        for line in resp.iter_lines():
            if not line:
                continue
            line = line.decode("utf-8")
            if line.startswith("data: "):
                data = line[6:]
                if data == "[DONE]":
                    break
                try:
                    event = json.loads(data)
                    # Extract text from various event types
                    if event.get("type") == "response.output_text.delta":
                        full_response += event.get("delta", "")
                    elif "delta" in event:
                        delta = event.get("delta", {})
                        if isinstance(delta, dict) and "text" in delta:
                            full_response += delta["text"]
                        elif isinstance(delta, str):
                            full_response += delta
                except json.JSONDecodeError:
                    continue
        
        return full_response.strip()
    
    except Exception as e:
        print(f"Request error: {e}", file=sys.stderr)
        return None


def main():
    config = load_config()
    
    parser = argparse.ArgumentParser(
        description="CLI tool using ChatGPT subscription for shell commands."
    )
    parser.add_argument("prompt", nargs="*", help="Prompt for command generation")
    parser.add_argument("--login", action="store_true", help="Authenticate with ChatGPT")
    parser.add_argument("--model", type=str, help="Model to use (e.g., gpt-5, gpt-5-codex)")
    parser.add_argument("--set-default-model", type=str, help="Set the default model")
    parser.add_argument("--info", action="store_true", help="Show auth status")
    
    args = parser.parse_args()
    
    if args.login:
        sys.exit(0 if do_login() else 1)
    
    if args.set_default_model:
        config["default_model"] = args.set_default_model
        save_config(config)
        print(f"Default model set to: {args.set_default_model}")
        sys.exit(0)
    
    if args.info:
        auth = load_auth()
        if auth:
            tokens = auth.get("tokens", {})
            access_token = tokens.get("access_token")
            if access_token:
                claims = parse_jwt_claims(access_token) or {}
                exp = claims.get("exp")
                if exp:
                    expiry = datetime.datetime.fromtimestamp(float(exp), datetime.timezone.utc)
                    print(f"Authenticated. Token expires: {expiry}")
                else:
                    print("Authenticated.")
            else:
                print("Auth file exists but no valid token.")
        else:
            print("Not authenticated. Run `python jose_gpt.py login`")
        sys.exit(0)
    
    if not args.prompt:
        print("Please provide a prompt.")
        sys.exit(1)
    
    model = args.model or config.get("default_model", "gpt-5-codex")
    prompt = " ".join(args.prompt)
    
    print(f"Querying {model}...")
    result = call_chatgpt(prompt, model)
    
    if result:
        # Get first line as main command
        lines = result.strip().splitlines()
        command = lines[0] if lines else result
        pyperclip.copy(command)
        print(f"Command copied to clipboard:")
        print(f"  {command}")
        if len(lines) > 1:
            print("Alternatives:")
            for alt in lines[1:]:
                if alt.strip():
                    print(f"  {alt}")
    else:
        print("Failed to get response.", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
