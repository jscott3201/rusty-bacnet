---
section: Added
---
- The CLI, the TUI and the MCP server take an object as `type,instance`
  (`analog-input,3`, `128,5`) beside `type:instance`, following the coming
  BACnet RDF encoding, so an identifier copied from an RDF model can be
  typed in; what they print is unchanged (#2473).
