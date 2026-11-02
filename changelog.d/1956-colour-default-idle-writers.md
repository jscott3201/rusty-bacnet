---
section: Changed
---
- A durable object's save thread now ends once the object has saved
  nothing for a while, and the next save starts another, so a device with
  many objects with storage no longer holds an idle thread for each (#1956).
