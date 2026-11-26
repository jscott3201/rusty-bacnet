---
section: Migration notes
---
- **Lifecycle errors (Rust and Python API, #1833):** match `Error::State`
  where you matched `Error::Transport` of kind `NotConnected` or
  `AlreadyExists`; `port::stopped()` is `NotRunning`, the new
  `port::stopped_for_good()` `Closed`. In Python, catch `BacnetStateError`
  (or `RuntimeError`) where a stopped link, or a B/IPv6 foreign device's
  broadcast before its BBMD resolves, raised `BacnetTransportError`, and
  `BacnetLookupError` for a `_from_device` call's unknown instance.
