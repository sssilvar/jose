# jose

A minimal CLI that turns a prompt into a shell command. **One prompt in, one command out** — copied straight to your clipboard.

Works with your ChatGPT account or any OpenAI-compatible server (Ollama, llama.cpp, vLLM, ...). Commands are generated for *your* exact OS, shell, and userland (GNU vs BSD flags, available package managers).

## Install

```bash
cargo install --git https://github.com/sssilvar/jose.git
```

## Usage

```bash
jose "delete all docker containers"      # generate a command
jose -m gpt-5.4 "find files over 1GB"    # one-off model override
jose info                                # auth status
```

```text
$ jose "git undo last commit but keep changes"
[+] Command copied to clipboard:
    git reset --soft HEAD~1
```

## Providers

### ChatGPT (default)

Authenticate once with your ChatGPT account (OAuth, same flow as Codex CLI). Credentials are stored in `~/.jose/auth.json`.

```bash
jose login
```

### OpenAI-compatible

Point jose at any `/v1` server. The API key is optional (Ollama and llama.cpp need none).

```bash
jose provider set openai-compatible --base-url http://localhost:11434/v1
jose provider set openai-compatible --base-url https://api.example.com/v1 --api-key sk-...
jose provider set chatgpt           # switch back

jose provider                       # show current provider
```

Environment variables `JOSE_BASE_URL` and `JOSE_API_KEY` override the config per-invocation.

## Models

```bash
jose model              # show current + known models
jose model set gpt-5.4  # set default (free-form for openai-compatible)
```

## License

MIT
