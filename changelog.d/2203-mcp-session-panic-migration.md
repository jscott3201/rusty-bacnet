---
section: Migration notes
---
- **TUI-attached listener (Rust API, #2203):** `bacnet_mcp::attach::Listener::bind`
  takes a `SessionRuntime`: `Own` runs the sessions on a runtime of the
  listener's own, `Callers` on the caller's (tests on a paused clock).
