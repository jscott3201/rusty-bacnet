---
section: Changed
---
- A damaged Audit Log snapshot slot (past a size or count cap, or holding a record that doesn't decode) now fails to load as an encoding error, and a record that won't encode is refused as out of range; Python still raises `BacnetError`, with new message text (#2047).
