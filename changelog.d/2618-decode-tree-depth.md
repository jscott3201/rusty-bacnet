---
section: Changed
---
- The frame decode tree counts nesting within each service parameter, so a
  value as deep as its decoder takes it keeps its named fields, and where a
  named service's tags don't frame it says `tag walk stopped: ...` as why
  it walked them tag by tag (#2618).
