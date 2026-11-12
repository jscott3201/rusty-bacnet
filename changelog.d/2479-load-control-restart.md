---
section: Added
---
- **Wire:** A Load Control built with `LoadControlObject::with_persistence`
  keeps its shed request across a restart and decides it again as the server
  starts; a peer's write of a request property is saved before it is
  answered, and refused when the save fails (#2479).
