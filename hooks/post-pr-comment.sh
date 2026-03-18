#!/bin/bash
# Sannai PR comment hook — Claude Code PostToolUse hook
# Detects `gh pr create` commands and posts provenance comments on the new PR.
#
# This closes the gap where sannai misses PRs created after pushing:
# the pre-push hook only fires at push time, so if the PR doesn't exist
# yet, sannai has no trigger. This hook catches PR creation directly.
#
# Install via: sannai hook install

INPUT=$(cat)
COMMAND=$(echo "$INPUT" | jq -r '.tool_input.command // empty')

# Only proceed if this was a gh pr create command
if ! echo "$COMMAND" | grep -qE '(^|&&|\|\||;)\s*gh\s+pr\s+create(\s|$)'; then
  exit 0
fi

CWD=$(echo "$INPUT" | jq -r '.cwd // empty')
if [ -z "$CWD" ]; then
  exit 0
fi

# Extract PR URL from tool output (handles both string and structured output)
PR_URL=$(echo "$INPUT" | jq -c '.tool_output' | grep -oE 'https://github\.com/[^/]+/[^/]+/pull/[0-9]+' | head -1)

if [ -z "$PR_URL" ]; then
  exit 0
fi

REPO=$(cd "$CWD" && git rev-parse --show-toplevel 2>/dev/null)
if [ -z "$REPO" ]; then
  exit 0
fi

SANNAI_BIN="${SANNAI_BIN:-sannai}"
SANNAI_LOG="${SANNAI_LOG:-/tmp/sannai-post-push.log}"

# Post comment in background (small delay to let GitHub process the PR)
(
  sleep 3
  echo "[$(date -Iseconds)] PR created, posting sannai comment on $PR_URL" >> "$SANNAI_LOG"
  if "$SANNAI_BIN" comment --pr "$PR_URL" --repo "$REPO" >> "$SANNAI_LOG" 2>&1; then
    echo "[$(date -Iseconds)] Success: comment posted on $PR_URL" >> "$SANNAI_LOG"
  else
    echo "[$(date -Iseconds)] Failed: sannai comment on $PR_URL" >> "$SANNAI_LOG"
  fi
) &
disown

exit 0
