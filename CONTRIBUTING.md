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
- Release notes and version bumps are generated from conventional commits.
  Keep squash-merge titles semantic so release-please can classify changes.

## Releases

[release-please](https://github.com/googleapis/release-please-action) manages
versions and changelogs. After all CI checks pass for a push to `main`, it
opens or updates a release PR. `fix:` commits bump the patch version, `feat:`
commits bump the minor version, and `!` or a `BREAKING CHANGE:` footer requests
a major version. Changes such as `ci:`, `chore:` and `docs:` alone do not
request a new release.

Review and merge the release PR when ready. It updates `Cargo.toml`,
`Cargo.lock`, `CHANGELOG.md` and `.release-please-manifest.json`; regular
feature and fix PRs should leave version bumps to the bot. Once the release
PR reaches `main` and passes CI, release-please creates its tag and release.
The CI workflow then calls `release.yml` directly to build the exact tag and
upload Linux/macOS archives and their SHA-256 checksums. Uploading preserves
the release notes generated from the changelog.

The initial manifest records `0.2.0`, and `bootstrap-sha` points to the
`v0.2.0` commit so the first automated changelog starts after that release.
The bootstrap setting can be removed after the first release-please PR has
been merged.

### Repository setup

Under **Settings → Actions → General → Workflow permissions**, enable
**Allow GitHub Actions to create and approve pull requests**. The workflow
uses the built-in `GITHUB_TOKEN` and declares write permissions only for
the release jobs; no additional token is required. It does not approve or
merge pull requests automatically.

For release PRs opened or updated with `GITHUB_TOKEN`, GitHub may show an
**Approve workflows to run** banner. A maintainer must approve those runs
before merging. See GitHub's
[workflow triggering rules](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow#triggering-a-workflow-from-a-workflow).
The bot's tag does not trigger a separate push workflow, which is why CI
calls the binary build directly.

### Retrying a binary build or upload

Open **Actions → Release → Run workflow**, choose `main`, and enter the
existing release tag (for example, `v0.2.1`). This rebuilds that tag and
replaces its binary assets without changing its version or release notes.
The tag must match the version in its `Cargo.toml`, and the GitHub release
must already exist. There is no need to delete or recreate the tag.

## License

By contributing you agree that your contributions are licensed under the
[Apache License 2.0](LICENSE), the same license as the project.
