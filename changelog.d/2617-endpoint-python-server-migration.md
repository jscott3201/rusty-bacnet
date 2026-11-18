---
section: Migration notes
---
- **Endpoint mutation policy (Python API, #2617):** `device_write_policy=f`
  becomes `mutation_policy=f`, asked about every mutation of every object. A
  0.15.0 callable such as `return context.source_network is None` then lets
  every local peer write any writable property (WriteProperty and each
  WritePropertyMultiple element), change the Device's Description, create
  and delete objects, and use the list services, AtomicWriteFile and
  WriteGroup.
