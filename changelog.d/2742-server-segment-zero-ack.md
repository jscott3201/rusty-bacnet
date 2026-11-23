---
section: Changed
---
- **Wire:** The full server reassembles segmented requests with the client's receiver: it
  acknowledges segment zero at once at any proposed window, and counts each window from its last
  acknowledgment, positive or negative (#2742).
