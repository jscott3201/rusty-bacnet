---
section: Changed
---
- **Wire:** COV-multiple contexts that report at once go out in one fixed
  order: by recipient, then subscriber process identifier, unconfirmed
  before confirmed. A subscriber with several contexts saw their reports in
  an order that varied from run to run (#2320).
