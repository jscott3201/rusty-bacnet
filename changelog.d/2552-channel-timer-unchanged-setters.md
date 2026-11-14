---
section: Fixed
---
- A Channel or Timer setter given the value the object already holds no longer
  counts as a configuration change, so a multi-write request to it is saved
  once instead of again at each later write (#2552).
