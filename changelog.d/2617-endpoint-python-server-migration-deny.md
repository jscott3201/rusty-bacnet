---
section: Migration notes
---
- **Endpoint mutation policy, the reverse (Python API, #2617):** a renamed
  callable that checked only `property_identifier` now allows that property
  on every object, not only the Device, and denies the COV subscriptions the
  read-only default allows. Check `object_identifier` too, allow what you
  mean to by `context.service`, and deny the rest, or leave
  `mutation_policy` out for the read-only server.
