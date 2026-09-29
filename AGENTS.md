# AGENTS.md

## Project

Porteiro is a keyboard-first terminal UI that discovers concrete aliases from
an OpenSSH client configuration and hands the selected alias to the user's
installed `ssh` binary. The name is branding only. Product copy and code use
standard SSH terminology without extending the name into a metaphor.

## Product contract

- Treat the SSH config as the only source of truth.
- Remain read-only: never rewrite SSH config files, keys, `known_hosts`, or
  agent state.
- Discover concrete aliases from `Host` directives, including files reached
  through `Include`; omit wildcard and negated patterns plus spellings that
  OpenSSH rejects or reinterprets as `user@host` or a lowercase `ssh://` URI.
  URI recognition is case-sensitive in OpenSSH; differently cased spellings
  remain literal aliases.
- Preserve declaration order and deduplicate aliases case-insensitively.
- Never execute or assume an outcome for stateful `Match` criteria while
  building the catalog. Treat their result and any first-value option they may
  set as unknown; retain only declarations reachable independently of them.
- On Unix, apply OpenSSH's owner and group/world-write checks to files reached
  through `Include`. The explicitly selected root `-F` file is not an include.
- Treat a reachable recursive `Include` cycle as invalid configuration;
  OpenSSH recurses until its depth guard and rejects it. A cycle encountered
  only through conservative candidate scanning may remain a warning until an
  exact alias traversal proves it reachable. Read and canonicalization failures
  from conservative-only candidates are warnings; the same failure on an exact
  matched path is an error. Directory-iteration errors produced while expanding
  a glob remain non-fatal because OpenSSH ignores those entries.
- Ask OpenSSH for effective details with `ssh -G`; do not reimplement its
  complete matching and precedence semantics.
- Treat catalog fields as declaration hints only; the details view from
  `ssh -G` is the effective result. Resolving a selected alias can evaluate
  user-authored `Match exec` directives and must retain its timeout.
- Disable hostname and CNAME canonicalization only for `ssh -G` previews so
  browsing does not trigger DNS. Never apply these overrides to connections.
- Connect by invoking the real `ssh` process with an argv vector. Never build a
  shell command or interpolate an alias into shell syntax.
- Restore the terminal successfully before starting an interactive SSH
  session. A restoration error must prevent process launch.
- Preserve normal SSH exit codes. On Unix, map signal termination to the
  conventional shell-visible status `128 + signal`.
- Do not perform automatic reachability checks or other network activity.

## Current scope

The surface is one full-screen host list with fuzzy search, collapsible groups,
tags, colors, responsive technical details, config reload, warnings, help, and
one-shot connection. Editing, favorites, recents, nested groups, persistent
application state, SFTP, and an embedded SSH implementation are out of scope.

Grouping metadata uses full-line `# porteiro:` comments in the SSH config.
File defaults precede the first local Host/Match; Host annotations override
them only for concrete aliases declared in that local block. Metadata never
flows through Include or participates in SSH matching. Keep metadata from the
retained reachable declaration, not an earlier conservative candidate. Invalid
annotations warn with file/line and apply no partial update. Reject control and
bidirectional formatting characters in metadata displayed by the terminal.

## Architecture

- `src/cli.rs`: command-line parsing and default config path resolution.
- `src/catalog.rs`: read-only candidate discovery, per-alias reachability
  validation, includes, declaration hints, and fuzzy match. Discovery may
  conservatively inspect target-dependent include paths, but only an alias
  reached by its own deterministic config pass enters the catalog. That pass
  tracks uncertain `Match` outcomes, OpenSSH matching case rules, normalized
  ProxyJump tokens, include permissions, the exact include traversal, and
  requested final-pass matching against the effective normalized host. A
  canonicalization pass uses the platform's DNS-free numeric-address APIs to
  recognize and format OpenSSH-compatible addresses, including legacy IPv4 and
  scoped IPv6; DNS-dependent host rewrites stay unknown. Honor
  `CanonicalizeMaxDots`, including its default and first-value semantics, when
  deciding whether a hostname can reach the final pass without DNS. With
  `CanonicalizeHostname yes`, a known ProxyJump or ProxyCommand makes the
  unchanged lowercase hostname deterministic, while `always` may still use
  DNS. Normalize valid ports before `%p` and `%C` token expansion. Keep
  tokenization aligned with OpenSSH's argument splitting: comments start only
  between tokens, unrecognized backslashes are data, and one unprotected
  leading `=` in the first value token is a separator rather than data. Keep an
  unknown target context for every positive wildcard `Host`, even when the same
  block also declares concrete aliases. Conservative target-token scans must be
  a filesystem superset across separators, hidden components, and lexical
  parent components within the containing configuration tree; exact per-alias
  validation remains the authority for admission. Widen components containing
  an unknown-token wildcard before glob matching so surrounding syntax cannot
  reinterpret the fallback as a literal. Follow distinct lexical symlink paths,
  while using canonical ancestors to stop directory cycles.
  Treat malformed glob syntax after expansion as an Include with no matches,
  as OpenSSH does; discovery may retain a diagnostic warning. Unsupported
  percent tokens and a trailing `%` are fatal syntax errors, while expansion
  failures that OpenSSH ignores, such as an unset environment variable, remain
  warnings.
- `src/catalog/metadata.rs`: atomic parsing of optional presentation annotations.
- `src/ssh.rs`: effective-config resolution and direct OpenSSH process launch.
- `src/ui/`: state machine, rendering, key handling, and terminal event loop.
  `list.rs` caches the grouped projection; rebuild it only when the filter,
  expansion or catalog changes. Group selection must never resolve or connect
  an SSH destination. Search expansion is temporary; keep normal expansion in
  memory only. Retain Ratatui's table state across renders for stable scrolling.
- `src/main.rs`: orchestration, terminal lifecycle, and exit status propagation.

Keep side effects outside the synchronous key handler. Key handling should
return an action, and the event loop should execute it. Keep parser and renderer
logic testable without a real terminal or network.

## User interface

- `j`/`k` or arrows: move selection.
- `/`: focus search; `Enter` accepts it and `Esc` clears it.
- `Enter`: connect to the selected alias or expand/collapse the selected group.
- `Space`: expand/collapse the selected group (ordinary text in search).
- `i`: open technical details as an overlay.
- `r`: reload the SSH config.
- `w`: inspect configuration warnings; `j`/`k` browse and `Esc` closes.
- `v`: cycle the `-v` verbosity of the next connection (off, 1, 2, 3). The
  selected row shows the flag as a badge, the `Enter` hint and status line
  repeat it, and `Enter` passes it as argv items.
- `?`: toggle help.
- `q` or `Ctrl-C`: quit.

Use direct technical labels such as Alias, HostName, User, Port, ProxyJump,
ProxyCommand, IdentityFile, and Config. Preserve readable behavior in narrow
terminals by dropping optional columns before truncating essential fields.

## Development

The supported toolchain is stable Rust. `nix develop` provides the complete
development shell when local Rust tooling is unavailable.

Run the full gate before committing. GitHub Actions runs the same gate on
Linux and macOS, plus a Windows `cargo check`, the MSRV check declared in
`Cargo.toml` and a dependency audit. After successful checks on `main`,
release-please manages release PRs, version bumps and changelogs. Merging a
release PR and passing CI creates its tag and release, then calls the reusable
release workflow to build binaries from that exact tag. Keep version changes
in the bot's release PR; use the Release workflow's tag input to retry assets.

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release --all-features
```

New behavior needs a happy-path test and a relevant edge or failure-path test.
Parser changes should cover quoting, comments, wildcard filtering, include
ordering, missing files, and cycles as applicable. UI changes should exercise
the state transition or render with Ratatui's test backend.

Use short semantic commits. Do not include generated build output or unrelated
working-tree changes.
