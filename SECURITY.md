# Security

Porteiro is designed to be safe to run against any SSH configuration you
already trust `ssh` with:

- It never writes: SSH config files, keys, `known_hosts` and agent state are
  read-only inputs.
- It never speaks the SSH protocol, probes hosts or performs network
  activity. Details come from `ssh -G` with hostname and CNAME
  canonicalization disabled, and the connection is the real `ssh` process.
- Aliases reach `ssh` as a single argv item. No shell is ever involved.
- Stateful `Match` criteria such as `exec` are never evaluated while building
  the catalog.
  OpenSSH itself does evaluate them during `ssh -G` for the selected alias,
  under a three-second timeout that kills the whole helper process group.
- Group, tag and color annotations are presentation-only comments. They cannot
  introduce an SSH destination or change the connection arguments. Malformed
  annotations are ignored with diagnostics; terminal control and bidirectional
  formatting characters are rejected in their values. Selecting a group does
  not launch `ssh -G`.

## Reporting a vulnerability

Please do not open a public issue for security problems. Use GitHub's
private vulnerability reporting on the repository's Security tab
("Report a vulnerability"). You will get an acknowledgement within a few
days and a fix or mitigation plan before any public disclosure.
