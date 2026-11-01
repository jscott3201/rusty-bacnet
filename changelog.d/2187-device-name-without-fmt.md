---
section: Changed
---
- `DeviceIdentity` writes the default Device and Network Port names
  (`device-<n>`, `port-<n>`) without `core::fmt`, so a no_std image that
  builds a Device database links no `core::fmt` (#2187).
