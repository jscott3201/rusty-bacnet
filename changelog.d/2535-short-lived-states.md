---
section: Fixed
---
- **Wire:** A state that a background pass makes and another writer undoes
  within one sweep period is now notified both ways, its entry and its
  return; before, the sweep's phase decided whether the first transition
  went out at all (#2535).
