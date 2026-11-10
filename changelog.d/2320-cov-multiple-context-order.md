---
section: Changed
---
- **Wire:** COV-multiple contexts that report at once go out in one fixed
  order: by recipient, then subscriber process identifier, unconfirmed
  before confirmed. Ahead of it go contexts a spent notification budget
  kept back last time. The order varied from run to run (#2320).
