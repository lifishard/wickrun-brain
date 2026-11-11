---
name: setup
description: Explain how to add a wickrun-brain profile (endpoint, model, key) safely from the user's own terminal, and what the tool changes. Use when the user wants to set up a new model, gateway, or API key for Claude Code.
---

Explain the steps below to the user. Do not run `add` yourself and never ask for, accept, or repeat a key in this chat: anything typed here is sent to the model provider. If the user pastes a key anyway, tell them to revoke it and create a new one.

Show the user the path of the tool on this machine (it is inside the plugin):

```
${CLAUDE_PLUGIN_ROOT}/scripts/wickrun-brain.mjs
```

Steps for the user, in their own terminal (not in this chat):

1. Put the key in an environment variable, for example in the shell profile they start Claude Code from:
   - macOS / Linux: `export MY_GATEWAY_KEY=...`
   - Windows PowerShell: `$env:MY_GATEWAY_KEY = "..."` (or set it as a user environment variable)
2. Save a profile that refers to the variable by name:
   `node "<path above>" add work --base-url https://gateway.example.com --model <model-id> --key-env MY_GATEWAY_KEY`
   Use `--auth x-api-key` if the gateway expects an `x-api-key` header instead of a bearer token.
   Alternative: `--key-stdin` reads the key from stdin (hidden input) and stores it in the profiles file.
3. Back in Claude Code, run `/wickrun-brain:switch work`, then start a new session.

What it changes: only these keys inside `env` of the user settings file: ANTHROPIC_BASE_URL, ANTHROPIC_AUTH_TOKEN, ANTHROPIC_API_KEY, ANTHROPIC_MODEL, ANTHROPIC_DEFAULT_OPUS_MODEL, ANTHROPIC_DEFAULT_SONNET_MODEL, ANTHROPIC_DEFAULT_HAIKU_MODEL, ANTHROPIC_DEFAULT_FABLE_MODEL, CLAUDE_CODE_SUBAGENT_MODEL. Before the first change it saves the original file, and `/wickrun-brain:restore` puts it back byte for byte.

You may run `node "${CLAUDE_PLUGIN_ROOT}/scripts/wickrun-brain.mjs" --help` and show its output verbatim. Do not read settings.json or the profiles file yourself.
