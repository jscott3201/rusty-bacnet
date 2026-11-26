---
section: Migration notes
---
- **First contact by instance (Rust, Python, CLI, MCP, #2824):** `Client::device(n)` and
  `client.device(n)` send one Who-Is and ask each claim first, and may fail as
  `OpsError::NotConfirmed` (`BacnetLookupError`, `at` the address asked) or `Ambiguous`.
  Match `NotConfirmed { answer, placing, .. }`. MCP refuses as `unconfirmed-device`
  with `claims`, and subscribes by instance only once a tool confirmed the device.
