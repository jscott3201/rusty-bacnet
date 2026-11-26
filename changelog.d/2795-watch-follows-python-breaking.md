---
section: Changed
---
- **Breaking (Python API):** a watch of a handle made from an instance
  follows its device instead of ending `"gone"`/`"moved"`; another device
  at its address makes it `"unreachable"`; `WatchUpdate.address` is the
  current address; an unconfirmed write raises `BacnetLookupError` (#2795).
