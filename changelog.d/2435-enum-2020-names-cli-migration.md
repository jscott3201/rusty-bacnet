---
section: Migration notes
---
- **Typed names (CLI, TUI, MCP, #2435):** type and expect `enable` for
  `log-enable`, `command` for `command-np`, `restart-autonegotiation` for
  `restart-autoneg`, `kiloohms` and `megaohms` for `kilohms` and `megohms`,
  and `segmented-both`, `segmented-transmit`, `segmented-receive` and
  `no-segmentation` for `both`, `transmit`, `receive` and `none`; the old
  names are refused. The TUI device list keeps its short labels.
