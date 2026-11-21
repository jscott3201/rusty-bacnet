---
section: Changed
---
- In bacnet-ops, a lookup by remote address that draws no answer now takes two resolve waits (6 s by default), one per Who-Is; a station known to answer only its network's Who-Is keeps that form after a lost answer (#2599).
