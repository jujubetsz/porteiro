# Porteiro

[![CI](https://github.com/jujubetsz/porteiro/actions/workflows/ci.yml/badge.svg)](https://github.com/jujubetsz/porteiro/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

Porteiro is a keyboard-first terminal UI for browsing hosts declared in your
OpenSSH client configuration and connecting with the `ssh` already installed on
your machine.

It treats the SSH config as the source of truth. Porteiro does not store
credentials, implement the SSH protocol, probe hosts, or modify configuration
files.

```text
 porteiro  OpenSSH config hosts
╭ Hosts - 2 shown / 2 total ─────────────────────────────────╮╭ Details ───────────────────────────╮
│  Alias                  HostName                     Port  ││Alias        production             │
│› production             prod.example.test            2222  ││HostName     prod.example.test      │
│  staging                staging.example.test         -     ││User         deploy                 │
│                                                            ││Port         2222                   │
│                                                            ││ProxyJump    bastion                │
│                                                            ││ProxyCommand -                      │
│                                                            ││IdentityFile ~/.ssh/id_rsa          │
│                                                            ││             ~/.ssh/id_ecdsa        │
│                                                            ││             ~/.ssh/id_ecdsa_sk     │
│                                                            ││             ~/.ssh/id_ed25519      │
│                                                            ││             ~/.ssh/id_ed25519_sk   │
╰────────────────────────────────────────────────────────────╯╰────────────────────────────────────╯
 2 host(s) | ~/.ssh/config
 j/k move  / search  Enter connect  i details  r reload  v verbose  ? help  q quit
```

## Features

- Reads `~/.ssh/config` by default and follows `Include` directives in
  declaration order, including target-dependent include paths.
- Lists concrete `Host` aliases that can be passed to `ssh` literally, omitting
  wildcard, negated and otherwise invalid spellings.
- Filters hosts with fuzzy search.
- Shows effective HostName, User, Port, ProxyJump, ProxyCommand and
  IdentityFile values from `ssh -G`.
- Restores the terminal and launches `ssh` directly when you press `Enter`,
  optionally with `-v`, `-vv` or `-vvv`, then returns the SSH exit status to
  your shell.
- Adapts columns and the details pane to the available terminal width.

## Install

Porteiro needs an OpenSSH-compatible `ssh` executable on the machine it runs on.

**Release binaries.** Download the archive for your platform from the
[releases page](https://github.com/jujubetsz/porteiro/releases), verify the
`.sha256` file, and place `porteiro` on your `PATH`:

```sh
tar -xzf porteiro-<version>-<target>.tar.gz
install -m 0755 porteiro-<version>-<target>/porteiro ~/.local/bin/porteiro
```

**From source** with a stable Rust toolchain (1.88 or newer):

```sh
cargo install --git https://github.com/jujubetsz/porteiro --locked
```

**Nix**, without installing anything:

```sh
nix run github:jujubetsz/porteiro
```

## Usage

```text
porteiro [--config PATH] [--ssh PATH]
```

`--config` selects another user configuration file. The same path is passed to
OpenSSH with `-F` for details and connections. `--ssh` selects another
OpenSSH-compatible executable and is useful when `ssh` is not on `PATH`.

| Key | Action |
| --- | --- |
| `j` / `k`, arrows | Move selection |
| `PgUp` / `PgDn`, `Home` / `End` | Move by page, jump to first or last host |
| `/` | Focus search |
| `Enter` | Accept search or connect |
| `Esc` | Clear search or close an overlay |
| `i` | Toggle the details overlay |
| `r` | Reload the config |
| `v` | Cycle the verbosity of the next connection: `-v`, `-vv`, `-vvv`, off |
| `?` | Toggle help |
| `q` / `Ctrl-C` | Quit |

The selected host shows a badge with the pending flag, and the `Enter` hint
and status line repeat it, so you can debug a failing connection without
leaving the list.

Porteiro exits after the SSH session ends and returns the SSH process exit
status to the calling shell. On Unix, a session terminated by a signal returns
the conventional shell-visible status `128 + signal`.

## How host discovery works

Values shown in the list are lightweight hints taken from each exact `Host`
block. The details view is authoritative: it asks OpenSSH for the selected
alias's effective configuration with `ssh -G` under a three-second timeout.

- OpenSSH evaluates user-authored `Match exec` directives while doing this, so
  commands intentionally present in the SSH config can run when a host is
  selected. Preview resolution disables hostname and CNAME canonicalization to
  avoid DNS traffic; the real connection uses the configuration unchanged.
- While building the catalog, command- and machine-state-dependent `Match`
  outcomes are treated as unknown, so they cannot introduce speculative aliases
  or execute local commands.
- Reachable recursive `Include` cycles are rejected, matching OpenSSH's
  eventual include-depth failure. On Unix, files reached through `Include`
  must satisfy OpenSSH's ownership and write-permission checks.
- Catalog validation models OpenSSH's requested final configuration pass:
  `Host` is matched against the effective normalized host while
  `originalhost` remains the selected alias. A pass requested by
  `CanonicalizeHostname` stays deterministic for literal IPs; hostnames whose
  canonical form may require DNS are treated as unknown. `CanonicalizeHostname
  yes` keeps a known proxied hostname unchanged because OpenSSH skips DNS in
  that case, and names with more dots than `CanonicalizeMaxDots` bypass
  canonicalization, matching OpenSSH's default and first-value semantics.
- Numeric addresses use the platform's DNS-free OpenSSH-compatible parser and
  canonical textual form, including legacy IPv4 spelling and scoped IPv6. Port
  values are normalized to decimal before `%p` and `%C` expansion.
- Aliases are the spellings OpenSSH accepts as a literal host argument:
  wildcard, negated, invalid, lowercase `ssh://` URI and `user@host` forms are
  omitted. OpenSSH treats the URI scheme case-sensitively, so a differently
  cased spelling remains a literal alias.
- Both spaced and unspaced `=` directive separators are accepted; a quoted
  leading `=` remains part of the value.

## Development

```sh
nix develop        # optional
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release --all-features
```

CI runs the same gate on Linux and macOS plus a Windows `cargo check`, a
minimum supported Rust version check and a dependency audit. Pushing a tag
`vX.Y.Z` that matches the version in `Cargo.toml` builds release binaries for
x86_64 Linux and Apple Silicon macOS and publishes them with generated release
notes.

See [CONTRIBUTING.md](CONTRIBUTING.md) for the workflow and
[AGENTS.md](AGENTS.md) for the product contract, invariants and architecture.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
