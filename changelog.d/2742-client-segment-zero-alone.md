---
section: Changed
---
- **Wire:** At a window above one, the client sends segment zero of a segmented request alone and
  takes the window from its answer. A peer leaving it unanswered (offline, slow, or never answering
  it alone) costs one segment timeout, then gets whole first windows until it answers at once (#2742).
