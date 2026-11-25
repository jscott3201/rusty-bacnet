---
section: Fixed
---
- A panic while a task holds one of the stack's internal locks no longer
  makes every later user of that lock panic too: the server, client,
  links, router and tools take it as the panic left it. `bacnet_types::sync`
  has the extension traits they use (#2559).
