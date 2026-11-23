---
section: Migration notes
---
- **Network rejects (Rust and Python, #2724):** code that waited for a
  timeout to learn that a routed request's router has no route to the
  network catches `Error::NetworkRejected` (Rust; `bacnet_ops` sorts it as
  `OpsError::Unreachable`, not a timeout) or `BacnetNetworkRejectError`
  (Python, not a `BacnetTimeoutError`) instead. A busy router (reason 2)
  still ends in a timeout.
