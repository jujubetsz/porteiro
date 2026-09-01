# Porteiro

Porteiro is a keyboard-first terminal UI for browsing hosts declared in your
OpenSSH client configuration and connecting with the `ssh` already installed on
your machine.

It treats the SSH config as the source of truth. Porteiro does not store
credentials, implement the SSH protocol, probe hosts, or modify configuration
files.

## Features

- Reads `~/.ssh/config` by default.
- Follows reachable OpenSSH `Include` directives and preserves declaration
  order, including target-dependent include paths whose alias expansion crosses
  directory separators, uses lexical parent components inside the configuration
  tree, starts a hidden path component, or appears inside glob syntax such as
  `[%n]`.
- Rejects reachable recursive `Include` cycles, matching OpenSSH's eventual
  include-depth failure.
- Treats command- and machine-state-dependent `Match` outcomes as unknown
  during browsing, so they cannot introduce speculative aliases or execute
  local commands.
- Lists concrete `Host` aliases whose spelling can be passed to `ssh` as a
  literal host argument, omitting wildcard, negated, invalid, lowercase
  `ssh://` URI, and `user@host` forms. OpenSSH treats the URI scheme
  case-sensitively, so a differently cased spelling remains a literal alias.
- Filters hosts with fuzzy search.
- Shows effective HostName, User, Port, ProxyJump, ProxyCommand, and
  IdentityFile values from `ssh -G`.
- Restores the terminal and launches `ssh` directly when you press `Enter`;
  restoration failure aborts the connection.
- Adapts columns and the details pane to the available terminal width.

## Requirements

- An OpenSSH-compatible `ssh` executable.
- A stable Rust toolchain to build from source, or Nix for the included
  development shell.

## Build

```sh
nix develop        # optional
cargo build --release
./target/release/porteiro
```

## Usage

```text
porteiro [--config PATH] [--ssh PATH]
```

`--config` selects another user configuration file. The same path is passed to
OpenSSH with `-F` for details and connections. `--ssh` selects another
OpenSSH-compatible executable and is useful when `ssh` is not on `PATH`.

Values shown in the list are lightweight hints taken from each exact `Host`
block. The details view is authoritative: it asks OpenSSH for the selected
alias's effective configuration with `ssh -G` and applies a three-second
timeout. OpenSSH evaluates user-authored `Match exec` directives while doing
this, so commands intentionally present in the SSH config can run when a host
is selected. Preview resolution disables hostname and CNAME canonicalization
to avoid DNS traffic; the real connection uses the configuration unchanged.
On Unix, files reached through `Include` must satisfy OpenSSH's ownership and
write-permission checks. Catalog validation also models OpenSSH's requested
final configuration pass: `Host` is matched against the effective normalized
host while `originalhost` remains the selected alias. A pass requested by
`CanonicalizeHostname` remains deterministic for literal IPs; hostnames whose
canonical form may require DNS are treated as unknown during catalog building.
Numeric addresses use the platform's DNS-free OpenSSH-compatible parser and
canonical textual form, including legacy IPv4 spelling and scoped IPv6.
Names containing more dots than `CanonicalizeMaxDots` bypass canonicalization
and remain deterministic, matching OpenSSH's default limit and first-value
semantics.
`CanonicalizeHostname yes` keeps a known proxied hostname unchanged because
OpenSSH skips DNS in that case. Port values are normalized to decimal before
`%p` and `%C` expansion. Both spaced and unspaced `=` directive separators are
accepted; a quoted leading `=` remains part of the value.

## Keys

| Key | Action |
| --- | --- |
| `j` / `k`, arrows | Move selection |
| `/` | Focus search |
| `Enter` | Accept search or connect |
| `Esc` | Clear search or close an overlay |
| `i` | Toggle the details overlay |
| `r` | Reload the config |
| `?` | Toggle help |
| `q` / `Ctrl-C` | Quit |

Porteiro exits after the SSH session ends and returns the SSH process exit
status to the calling shell. On Unix, a session terminated by a signal returns
the conventional shell-visible status `128 + signal`.

## Development

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release --all-features
```

See `AGENTS.md` for project invariants and architecture.
