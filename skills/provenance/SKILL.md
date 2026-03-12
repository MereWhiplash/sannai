---
name: sannai-provenance
description: This skill should be used when the user asks to "post provenance", "add sannai comment", "add provenance to PR", "run sannai", or after creating a pull request when sannai is available. Posts AI coding session provenance comments on GitHub PRs using the sannai CLI.
---

# Sannai Provenance

Post AI coding session provenance on GitHub pull requests using the `sannai` CLI.

**Announce:** "Posting sannai provenance comment on the PR."

## When to Use

- After creating or pushing to a pull request
- When the user explicitly asks for provenance or sannai comments
- When wrapping up a branch that will become a PR

## Prerequisites

Before posting provenance, verify the environment:

```bash
sannai status
```

Expected: daemon running, sessions captured. If sannai is not running or not installed, inform the user and skip — do not attempt to install or start it.

Also verify `gh` CLI is authenticated:

```bash
gh auth status
```

## Posting Provenance

### Step 1: Verify Sessions Exist

```bash
sannai sessions
```

Look for sessions matching the current project path. If no sessions are found, inform the user that sannai may not have been running during development.

### Step 2: Post the Comment

```bash
sannai comment --pr <PR_URL>
```

The PR URL must be the full GitHub URL, e.g. `https://github.com/owner/repo/pull/123`.

If the PR was just created in this session, reuse the URL returned by `gh pr create`.

### Step 3: Confirm

Report success or failure to the user. If the command fails, check:
- Is `gh` authenticated? (`gh auth status`)
- Does the PR exist? (`gh pr view <url>`)
- Are there sessions for this repo? (`sannai sessions`)

## Integration with PR Workflow

When used after creating a PR, the typical flow is:

```
1. Finish work, commit
2. Push branch
3. gh pr create → get PR URL
4. sannai comment --pr <PR URL>
```

Combine steps 3 and 4 when the PR URL is available. Do not prompt the user between these steps — just run both.

## What Gets Posted

The sannai comment includes:
- Session count and duration
- Interaction breakdown (prompts, tool calls)
- File lineage (which files were read/written)
- Diff attribution (which PR changes map to which AI interactions)
- Optional LLM-generated summary (if configured)
