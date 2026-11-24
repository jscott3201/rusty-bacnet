---
section: Changed
---
- **Wire:** At a window above one, the client sends segment zero of a segmented request alone and
  takes the window from its answer. A peer that never answers it alone costs one segment timeout,
  once (a peer that drops the repeat fails that request); after that it gets the whole first window
  (#2742).
