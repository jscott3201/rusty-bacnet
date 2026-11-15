---
section: Added
---
- **Python API:** `BACnetClient.relinquish(address, object_id, priority)`
  releases one command slot and returns the slots read back, and
  `priority_array(address, object_id)` reads them, as a `PriorityView` of
  `Value`s (#2561).
