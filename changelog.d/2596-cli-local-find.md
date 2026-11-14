---
section: Changed
---
- **Breaking (CLI):** `bacnet find` sends its Who-Has on this network alone
  unless the new `--dnet`, `--global` or `--target` says where else, and a
  miss there notes on stderr how to widen it; the TUI's `:find` takes
  `--dnet N` and `--global` (#2596).
