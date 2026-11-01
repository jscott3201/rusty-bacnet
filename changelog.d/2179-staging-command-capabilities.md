---
section: Changed
---
- `BACnetObject`'s hidden Staging and Command/Channel run hooks move behind
  two hidden capability accessors, `staging_source()` and
  `command_runner()`/`command_runner_mut()`; the public API is unchanged
  (#2179).
