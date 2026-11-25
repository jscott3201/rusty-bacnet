---
section: Fixed
---
- A segmented request to a peer that never acknowledges now ends with a
  timeout at the largest retry setting (255) too, where its retry count
  overflowed (#2809).
