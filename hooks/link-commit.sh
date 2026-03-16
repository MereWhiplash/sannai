#!/bin/bash
# Sannai commit linker — Claude Code PostToolUse hook
# Detects git commit commands and links them to the active session via sannai API.
#
# Install: Add to ~/.claude/settings.json or .claude/settings.json:
# {
#   "hooks": {
#     "PostToolUse": [{
#       "matcher": "Bash",
#       "hooks": [{ "type": "command", "command": "/path/to/sannai/hooks/link-commit.sh" }]
#     }]
#   }
# }

INPUT=$(cat)
COMMAND=$(echo "$INPUT" | jq -r '.tool_input.command // empty')

# Only proceed if this was a git commit command
if ! echo "$COMMAND" | grep -qE '(^|&&|\|\||;)\s*git\s+commit(\s|$)'; then
  exit 0
fi

CWD=$(echo "$INPUT" | jq -r '.cwd // empty')
SESSION_ID=$(echo "$INPUT" | jq -r '.session_id // empty')

if [ -z "$CWD" ] || [ -z "$SESSION_ID" ]; then
  exit 0
fi

# Get the commit SHA and repo root
SHA=$(cd "$CWD" && git rev-parse HEAD 2>/dev/null)
REPO=$(cd "$CWD" && git rev-parse --show-toplevel 2>/dev/null)

if [ -z "$SHA" ] || [ -z "$REPO" ]; then
  exit 0
fi

# Link the commit to the session via sannai API
PAYLOAD=$(jq -n --arg sha "$SHA" --arg repo "$REPO" --arg sid "$SESSION_ID" \
  '{sha: $sha, repo: $repo, session_id: $sid}')

curl -s -X POST http://127.0.0.1:9847/hook/commit \
  -H "Content-Type: application/json" \
  -d "$PAYLOAD" \
  > /dev/null 2>&1

exit 0
