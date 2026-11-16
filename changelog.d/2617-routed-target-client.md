---
section: Added
---
- **Python API:** `BACnetClient`'s `read_property`, `read_property_multiple`,
  `read_range`, `read_log_page` and `write_property` take a `RoutedTarget`;
  with no `router` they use the route the client learns (#2617).
