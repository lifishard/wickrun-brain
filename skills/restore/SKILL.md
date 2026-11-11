---
name: restore
description: Put Claude Code's settings.json back to exactly the bytes it had before wickrun-brain first changed it, then delete the backup.
disable-model-invocation: true
---

Run this command and show its output verbatim in a code block:

```
node "${CLAUDE_PLUGIN_ROOT}/scripts/wickrun-brain.mjs" restore
```

Then tell the user to start a new Claude Code session for the change to take effect.

Rules:
- `restore` also discards any edits made to settings.json after wickrun-brain first changed it. If the user only wants to drop the endpoint and model overrides and keep later edits, run `node "${CLAUDE_PLUGIN_ROOT}/scripts/wickrun-brain.mjs" official` instead.
- Never add `--force` unless the user explicitly asks to override the wickrunAI desktop app after seeing its message.
- Do not open, read, edit, or print `settings.json` or the backup file yourself.
