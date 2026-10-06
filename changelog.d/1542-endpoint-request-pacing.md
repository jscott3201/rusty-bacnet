---
section: Added
---
- **Breaking (Rust API):** the endpoint client paces confirmed requests to each
  device as `BACnetClient` does, through `SessionConfig::min_request_interval_ms`
  and the endpoint builders' `min_request_interval_ms` (default 0, at most an
  hour) (#1542).
