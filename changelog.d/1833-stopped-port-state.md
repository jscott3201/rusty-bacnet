---
section: Changed
---
- **Breaking (Rust API):** a send on a transport that isn't started or has
  stopped, and a second start, return `Error::State` instead of
  `Error::Transport`: the source Audit no longer counts such a send as
  attempted, and `bacnet-ops` treats it as a local failure, not an
  unreachable device (#1833).
