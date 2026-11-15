---
section: Migration notes
---
- **bacnet find (CLI and TUI, #2596):** add `--global` to search every
  network as before, or `--dnet N` for one network; in the TUI write
  `:find --global NAME`, and quote a name that starts with `--`.
