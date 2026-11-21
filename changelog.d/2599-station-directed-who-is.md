---
section: Added
---
- Python’s `who_is` and `discover` accept `scope=RoutedTarget(...)`. The Who-Is keeps its station’s network and MAC even without a known router. Rust’s `BACnetClient::localized_device_snapshot()` returns one row per station and the local network number used (#2599).
