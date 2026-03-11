# Contributing to Sannai

Thanks for your interest in contributing to Sannai!

## Prerequisites

- [Rust](https://rustup.rs/) stable toolchain
- SQLite3 development libraries (usually pre-installed on macOS/Linux)

## Build from Source

```bash
git clone https://github.com/MereWhiplash/sannai.git
cd sannai
make build
```

Or directly:

```bash
cd agent && cargo build
```

## Running Tests

```bash
make test
```

Or:

```bash
cd agent && cargo test
```

## Code Style

- Format with `cargo fmt` (uses project `rustfmt.toml`)
- Lint with `cargo clippy -- -D warnings`
- Run `make lint` to check both

## Pull Request Process

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/my-change`)
3. Make your changes
4. Ensure tests pass (`make test`)
5. Ensure lints pass (`make lint`)
6. Commit with a clear message
7. Open a pull request against `main`

## Reporting Issues

- Use the [bug report template](https://github.com/MereWhiplash/sannai/issues/new?template=bug_report.yml) for bugs
- Use the [feature request template](https://github.com/MereWhiplash/sannai/issues/new?template=feature_request.yml) for ideas
- Check existing issues before filing a new one

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
