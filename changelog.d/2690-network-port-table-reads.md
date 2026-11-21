---
section: Added
---
- **Python API:** whole reads of a Network Port's BBMD_Broadcast_Distribution_Table and BBMD_Foreign_Device_Table are tagged `bdt` and `fdt`, each entry a mapping whose address text matches `read_bdt()` and `read_fdt()` for IPv4. The CLI, TUI and MCP show both tables decoded too (#2690).
