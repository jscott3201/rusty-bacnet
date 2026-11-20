---
section: Migration notes
---
- **Breaking (Rust API).** Add `retry_forever: false` to complete `ScReconnectConfig` literals to keep finite retries. Set it to `true` for continuous recovery. Python `ScReconnect` accepts the same optional keyword.
