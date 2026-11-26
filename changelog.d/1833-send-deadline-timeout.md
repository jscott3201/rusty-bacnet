---
section: Changed
---
- **Breaking (Rust API):** an endpoint send, or a server's audit forwarding,
  that this side's link didn't carry before its deadline is an
  `Error::Transport` of kind `TimedOut`, not `Error::Encoding` text and not
  the timeout that says a device gave no answer (#1833).
