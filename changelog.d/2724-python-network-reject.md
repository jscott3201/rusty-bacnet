---
section: Changed
---
- **Python API:** a request to a `RoutedTarget` that its router refuses
  raises `BacnetNetworkRejectError`, with a new `router` attribute in the
  link's address form, instead of a `BacnetAbortError` for a timeout
  (#2724).
