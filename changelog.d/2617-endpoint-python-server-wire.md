---
section: Changed
---
- **Breaking (wire):** a Python endpoint answers Who-Is, Who-Has and every
  service a standalone server executes, where it rejected all but
  ReadProperty; without a `mutation_policy` it denies each write, create and
  delete and ignores time synchronization (#2617).
