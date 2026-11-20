---
section: Added
---
- Direct a Who-Is to a remote station with Python’s `who_is(scope=RoutedTarget(...))` or `discover(scope=RoutedTarget(...))`. The request keeps its destination network and MAC even when the client has yet to learn a router. Rust’s `BACnetClient::localized_device_snapshot()` returns the local network number and one row per station as seen with that number (#2599).
