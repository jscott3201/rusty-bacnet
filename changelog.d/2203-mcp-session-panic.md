---
section: Fixed
---
- **Breaking (Rust API):** a bug an attached agent's MCP request reaches no
  longer stops the TUI drawing: the sessions run on the listener's own
  threads, and a panic there ends only that agent's session, audited
  (#2203). `Listener::bind` says where sessions run.
