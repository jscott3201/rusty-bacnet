---
section: Added
---
- **Wire:** A Load Control built with `LoadControlObject::with_persistence`
  keeps its shed request, Enable and a written Description across a restart,
  decides the request again as the server starts and stops one shed before
  it; a peer's write of these is saved before it is answered (#2479).
