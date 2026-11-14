---
section: Changed
---
- **Breaking (CLI):** `bacnet discover` with no `--dnet`, `--target` or the
  new `--global` sends a local broadcast, which no router passes on, instead
  of a global one (#2560).
