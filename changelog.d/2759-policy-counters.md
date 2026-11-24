---
section: Changed
---
- **Breaking (Rust API):** `EndpointSession::policy_counters` is a plain
  method, no longer `async`: it never waits (#2759).
