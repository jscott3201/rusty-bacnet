---
section: Changed
---
- Log and Audit Log snapshots save and load with fewer allocations and a table-driven checksum, and a log dropped before its first save commits without starting a writer thread (#2233).
