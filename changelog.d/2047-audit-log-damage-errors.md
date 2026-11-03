---
section: Changed
---
- A damaged Audit Log snapshot slot (past the size cap, or holding a capacity, record count, record or receipt past its cap) now fails to load as an encoding error rather than out of range; Python still raises `BacnetError`, with new message text (#2047).
