---
section: Fixed
---
- **Wire:** A confirmed event notification to a Device recipient that waited
  for an invoke ID now reads the binding again: a refreshed one still takes
  it, a moved one gets it at the new address, and a lapsed one is looked for
  with a Who-Is instead of being dropped (#2569).
