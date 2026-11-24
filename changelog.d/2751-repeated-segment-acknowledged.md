---
section: Fixed
---
- **Wire:** The full server acknowledges again a segment that a requester repeats while its
  reassembled request still executes, instead of answering it with an Abort that could reach the
  requester before the answer (#2751).
