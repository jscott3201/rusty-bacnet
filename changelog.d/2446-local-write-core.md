---
section: Added
---
- `bacnet-device-core`'s `write::local::apply` makes the application's own
  change to an object of its device, an input or a write as the local
  Device or one of its objects, under the host's guard; `bacnet-server`'s
  `write_local` and other `*_local` methods now make theirs through it
  (#2446).
