---
section: Added
---
- **Python API:** `Device.relinquish(object, priority)` releases one command slot and returns the slots read back, and `Device.priority_array(object)` reads them, as a `PriorityView` of `Value`s (#2561).
