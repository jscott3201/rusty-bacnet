---
section: Changed
---
- **Python API:** a request to a `RoutedTarget` that its router refuses
  raises `BacnetNetworkRejectError`, with new `router` and `reached_device`
  attributes, instead of a `BacnetAbortError` for a timeout (#2724).
