---
section: Changed
---
- `DeviceIdentity::new` writes its default Object_Name, `device-<instance>`,
  without `core::fmt`, so no_std images that build a Device format one
  string fewer (#2187).
