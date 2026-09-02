# Contributing

Thanks for helping with Porteiro. This file covers the mechanics; the
product contract, architecture and invariants live in [AGENTS.md](AGENTS.md)
and apply to every change.

## Setup

Use a stable Rust toolchain (`rust-toolchain.toml` selects it and adds
`clippy` and `rustfmt`), or enter the Nix development shell with
`nix develop`. An OpenSSH-compatible `ssh` is needed to run the TUI itself,
not to build or test it.

## Before opening a pull request

Run the full gate. CI runs the same commands on Linux and macOS.

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release --all-features
```

New behavior needs a happy-path test and a relevant edge or failure-path
test. Parser changes should cover quoting, comments, wildcard filtering,
include ordering, missing files and cycles as applicable. UI changes should
exercise the state transition or render with Ratatui's test backend.

## Commits and pull requests

- Use short semantic commit messages (`feat:`, `fix:`, `refactor:`, `docs:`,
  `ci:`, `build:`), one logical change per commit.
- Keep pull requests focused. Do not include generated build output or
  unrelated working-tree changes.
- Release notes are generated from merged pull requests and commits, so a
  clear title matters.

## License

By contributing you agree that your contributions are licensed under the
[Apache License 2.0](LICENSE), the same license as the project.
