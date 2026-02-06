# jose

A minimal CLI that generates shell commands using your ChatGPT account.

**One prompt in, one command out.** No complex TUI, no chat interface—just the command you need, copied to your clipboard.

## Installation

### From source (recommended)

```bash
cargo install --git https://github.com/sssilvar/jose.git
```

### From local clone

```bash
git clone https://github.com/sssilvar/jose.git
cd jose
cargo install --path .
```

## Setup

Before using jose, authenticate with your ChatGPT account:

```bash
jose login
```

This opens your browser for OAuth authentication. Your credentials are stored securely in `~/.jose/auth.json`.

## Usage

```bash
# Generate a command
jose "delete all docker containers"

# Use a specific model
jose -m gpt-5 "find large files over 1GB"

# Check authentication status
jose info

# Set default model
jose set-model gpt-5-codex
```

## Examples

```bash
$ jose "git undo last commit but keep changes"
[+] Command copied to clipboard:
    git reset --soft HEAD~1

$ jose "compress folder to tar.gz"
[+] Command copied to clipboard:
    tar -czvf folder.tar.gz folder/

$ jose "list open ports"
[+] Command copied to clipboard:
    lsof -i -P -n | grep LISTEN
```

## How it works

jose uses the same OAuth flow as OpenAI's Codex CLI to authenticate with your ChatGPT account. Commands are generated using the ChatGPT API and automatically copied to your clipboard.

## License

MIT
