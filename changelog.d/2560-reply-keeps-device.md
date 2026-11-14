---
section: Fixed
---
- A device that keeps answering the client's confirmed requests stays in the
  discovery table without repeating its I-Am; before, the purge dropped it 10
  to 15 minutes after its last I-Am, and calls by its instance failed (#2560).
