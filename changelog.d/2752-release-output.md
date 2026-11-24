---
section: Changed
---
- `ObjectDatabase::release_outputs` calls the new `Timed::release_output`, which lets go of what a start drove and keeps the object's state: a Load Control keeps its request across a refused start (#2752).
