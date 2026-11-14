---
section: Changed
---
- MCP tools that take a device also take the address of one in the
  discovery table; `dnet:instance` names the claim on that network, and two
  networks holding one instance are two devices to the policy and the
  caches. Nothing is sent to resolve a target (#2560).
