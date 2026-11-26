---
section: Migration notes
---
- **First contact by instance (Rust, Python, CLI, MCP, #2824):** `Client::device(n)`
  and `client.device(n)` now ask the device first and may raise
  `OpsError::NotConfirmed` / `BacnetLookupError` (`at` is the address asked) where
  the first request used to go out; MCP refuses as `unconfirmed-device`. Retry
  once the device answers, or name it by its address to reach whatever is there.
