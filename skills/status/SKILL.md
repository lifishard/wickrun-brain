---
name: status
description: Show which model ("brain") and endpoint Claude Code is configured to use, whether settings.json overrides the official login, and who is in control (this tool, the wickrunAI desktop app, or nobody). Use when the user asks what model or endpoint Claude Code is using.
---

Run this command and show its output to the user verbatim, in a code block:

```
node "${CLAUDE_PLUGIN_ROOT}/scripts/wickrun-brain.mjs" status
```

Rules:
- Do not open, read, or print `settings.json` or the profiles file yourself. The command already masks keys.
- Never ask the user for an API key or token.
- If the output says the wickrunAI desktop app is in control, tell the user to switch or restore from the wickrunAI app.
- The status describes what new Claude Code sessions will use; this running session may still be on its old settings.
