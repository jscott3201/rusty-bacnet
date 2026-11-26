---
section: Migration notes
---
- **Requests cut off by their link (Rust and Python API, #1833):** a request
  whose retry or later segment a stopped link refused, or a BVLC management
  request whose wait a stop dropped, is `Error::RequestCutOff { sent: true }`:
  don't repeat a write on it blindly. In Python catch `BacnetError` for it
  (`BacnetTransportError` before) and read the message for whether it went out;
  the fixed "the client stopped while it waited" text is gone.
