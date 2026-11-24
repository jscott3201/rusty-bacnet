---
section: Changed
---
- **Breaking (CLI):** an unknown `--format`, or a format the command doesn't
  print (`csv` outside `inventory`; `--format` or `--json` on `export`, `tui`
  or `mcp`), is a usage error (status 2) naming the formats it prints, not
  the default output (#2778).
