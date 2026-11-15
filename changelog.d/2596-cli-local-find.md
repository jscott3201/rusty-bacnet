---
section: Changed
---
- **Breaking (CLI):** `bacnet find` and the TUI's `:find` ask this network
  alone by default; `find` takes `--dnet`, `--global` and `--target`, `:find`
  takes `--dnet N` and `--global`, a local miss notes how to widen it, and
  each row now carries the address it answered from (#2596).
