---
name: switch
description: Switch Claude Code to one of the user's saved wickrun-brain profiles, or back to the official login. Use when the user asks to change the model, endpoint, or gateway Claude Code uses.
argument-hint: "[profile-name | official]"
disable-model-invocation: true
---

The user asked to switch to: $ARGUMENTS

1. Run this command and show its output verbatim in a code block:

   ```
   node "${CLAUDE_PLUGIN_ROOT}/scripts/wickrun-brain.mjs" list
   ```

2. Decide the target:
   - If the user asked for the official login, run `node "${CLAUDE_PLUGIN_ROOT}/scripts/wickrun-brain.mjs" official`.
   - Otherwise the target must be one of the profile names printed by `list`, spelled exactly. If no name was given or it does not match, ask the user which listed profile they want. Do not guess and do not invent a name.
   - Then run `node "${CLAUDE_PLUGIN_ROOT}/scripts/wickrun-brain.mjs" use <name>` with that exact name.
3. Show the output verbatim. Then tell the user to start a new Claude Code session for the change to take effect.

Rules:
- Never add `--force` unless the user explicitly asks to override the wickrunAI desktop app after seeing its message.
- If there are no profiles, explain that profiles are added from the user's own terminal (see `/wickrun-brain:setup`). Never ask the user to paste a key into this chat.
- Do not open, read, or print `settings.json` or the profiles file yourself.
