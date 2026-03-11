# Open Source Release (GH18)

## Summary

Prepare sannai for open-source release as a local-only Rust agent. Scope: OSS boilerplate files, crates.io packaging, and native daemon management (launchd/systemd).

Homebrew formula deferred to a future issue.

## Decisions

### Crate name: `sannai`
Binary is already `sannai`. Rename crate from `sannai-agent` to `sannai` in Cargo.toml. `cargo install sannai`.

### Version: 0.1.0
Pre-1.0 semver. First tagged release will be `v0.1.0`.

### GitHub auth: auto-detect from `gh` CLI
`sannai comment --pr` uses `gh auth token` output. No separate token config needed. Error message guides users to install/auth `gh` if missing.

### Uninstall: keep data by default
`sannai uninstall` removes the service file only. `sannai uninstall --purge` also removes data dir (SQLite, watcher state, PID file).

### No `sannai init` command
`sannai start` already auto-creates data dir and SQLite. `sannai install` is the "set and forget" path for daemon management.

## Open Source Boilerplate

| File | Content |
|------|---------|
| `LICENSE` | MIT full text (year 2026, Sannai contributors) |
| `CONTRIBUTING.md` | Build from source, run tests, PR process, code style (rustfmt, clippy) |
| `CODE_OF_CONDUCT.md` | Contributor Covenant v2.1 |
| `.github/ISSUE_TEMPLATE/bug_report.yml` | YAML form: description, steps to reproduce, expected, OS/version |
| `.github/ISSUE_TEMPLATE/feature_request.yml` | YAML form: problem, proposed solution, alternatives |
| README badges | CI status, license, crate version |

## Packaging (crates.io)

Update `agent/Cargo.toml`:
- `name = "sannai"`
- Add: `repository`, `homepage`, `keywords`, `categories`, `readme`
- Verify: `cargo package --list` shows no junk

## Install Experience

### `sannai install`

Detects OS and writes a service file:

**macOS** — `~/Library/LaunchAgents/dev.sannai.agent.plist`
- `RunAtLoad: true`, `KeepAlive: true`
- Binary path from `std::env::current_exe()`
- Args: `start --foreground`
- Stdout/stderr: `~/Library/Logs/sannai.log`
- Loads via `launchctl load`

**Linux** — `~/.config/systemd/user/sannai.service`
- `WantedBy=default.target`, `Restart=on-failure`, `RestartSec=5`
- Binary path from `std::env::current_exe()`
- ExecStart with `start --foreground`
- Enables via `systemctl --user enable --now sannai`

### `sannai uninstall`

- Stops the service (`launchctl unload` / `systemctl --user disable --now`)
- Removes the service file
- `--purge` flag: also removes data dir

### `sannai status` (updated)

Currently shows PID. Enhance to also show:
- Whether installed as service (launchd/systemd)
- Data dir location
- SQLite session count

## Implementation Plan

See: `docs/plans/2026-03-11-open-source-release.md`
