---
section: Changed
---
- **Breaking (wire):** a confirmed request to a station on another network
  whose MAC is that network's broadcast (one octet `0xFF`, or three or six
  octets of `0xFF`) is refused before anything is sent; bacnet-ops
  addresses and the CLI, TUI and MCP targets refuse all three (#2709).
