# SANNAI-MANAGED-HOOK
# SANNAI-MANAGED-HOOK
#!/bin/bash
# Sannai post-push hook — auto-posts provenance comments on open PRs.
#
# This is a git pre-push hook that fires sannai comment in the background
# AFTER the push completes. It detects if an open PR exists for the branch
# being pushed and runs `sannai comment` with LLM summary generation.
#
# Install (symlink into any repo):
#   ln -sf /Users/ashortt/dev/sannai/hooks/post-push-comment.sh .git/hooks/pre-push
#
# Or install globally:
#   git config --global core.hooksPath /Users/ashortt/dev/sannai/hooks/global
#
# The hook reads stdin for push refs (standard git pre-push protocol),
# lets the push proceed, then backgrounds the comment job.

SANNAI_BIN="${SANNAI_BIN:-/Users/ashortt/dev/sannai/agent/target/debug/sannai}"
SANNAI_LOG="${SANNAI_LOG:-/tmp/sannai-post-push.log}"

# pre-push receives: <remote-name> <remote-url>
REMOTE="$1"
REMOTE_URL="$2"

# Read the push refs from stdin
# Format: <local ref> <local sha> <remote ref> <remote sha>
while read -r LOCAL_REF LOCAL_SHA REMOTE_REF REMOTE_SHA; do
    # Extract branch name from ref
    BRANCH="${LOCAL_REF#refs/heads/}"

    # Skip deletes (local sha is all zeros)
    if echo "$LOCAL_SHA" | grep -qE '^0+$'; then
        continue
    fi

    # Detect the GitHub owner/repo from the remote URL
    OWNER_REPO=""
    if echo "$REMOTE_URL" | grep -q "github.com"; then
        # https://github.com/owner/repo.git or git@github.com:owner/repo.git
        OWNER_REPO=$(echo "$REMOTE_URL" | sed -E 's|.*github\.com[:/]([^/]+/[^/.]+)(\.git)?$|\1|')
    fi

    if [ -z "$OWNER_REPO" ]; then
        continue
    fi

    # Background: wait for push to complete, then check for PR and comment
    (
        # Small delay to ensure the push has landed on the remote
        sleep 3

        # Check if there's an open PR for this branch
        PR_NUMBER=$(gh pr view "$BRANCH" --repo "$OWNER_REPO" --json number --jq '.number' 2>/dev/null)

        if [ -z "$PR_NUMBER" ]; then
            echo "[$(date -Iseconds)] No open PR for branch '$BRANCH' on $OWNER_REPO, skipping" >> "$SANNAI_LOG"
            exit 0
        fi

        PR_URL="https://github.com/$OWNER_REPO/pull/$PR_NUMBER"
        REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null)

        echo "[$(date -Iseconds)] Posting sannai comment on $PR_URL" >> "$SANNAI_LOG"

        if "$SANNAI_BIN" comment --pr "$PR_URL" --repo "$REPO_ROOT" >> "$SANNAI_LOG" 2>&1; then
            echo "[$(date -Iseconds)] Success: comment posted on $PR_URL" >> "$SANNAI_LOG"
        else
            echo "[$(date -Iseconds)] Failed: sannai comment on $PR_URL" >> "$SANNAI_LOG"
        fi
    ) &

    # Disown so the background job doesn't block the push
    disown
done

# Always exit 0 — never block a push
exit 0
