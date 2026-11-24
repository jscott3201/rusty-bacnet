---
section: Migration notes
---
- **--format (CLI, #2778):** a value other than `table` or `json` (or `csv`
  for `inventory`) used to print the default, a table at a terminal and JSON
  through a pipe; give `table` or `json`, or leave the flag out for that
  default. Drop `--format` and `--json` from `export 223-experimental`, `tui`
  and `mcp` command lines.
