---
section: Migration notes
---
- **Following watches and writes (Python API, #2795):** a watch from a
  handle made from an instance no longer ends `"gone"` with `why.kind ==
  "moved"` when the device moves: read `update.moved` and key your points by
  object and property, not by `update.address`, which can change
  mid-stream. Treat `"unreachable"` with `why.kind == "replaced"` (and
  `"unconfirmed"`) as the watch looking for its device. Catch
  `BacnetLookupError` from a write on such a handle, read
  through it, and write again.
