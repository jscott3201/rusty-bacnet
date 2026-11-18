---
section: Migration notes
---
- **Endpoint mutation policy, the reverse (Python API, #2617):** a renamed
  callable that checked `property_identifier` now denies the COV
  subscriptions the read-only default allows. Have the callable deny what it
  does not recognise and allow what it means to by `context.service`, or
  leave `mutation_policy` out for the read-only server.
