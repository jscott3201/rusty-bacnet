---
section: Added
---
- `ErrorCode` names the BACnet/SC connection codes 152 to 199 (HTTP,
  WebSocket, TLS, DNS, TCP and IP), so the CLI, TUI, MCP server and Python
  show them by name, and `ScDisconnectCause::error_code()` gives the code for
  a hub connection's failure (#2434).
