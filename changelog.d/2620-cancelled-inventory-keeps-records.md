---
section: Fixed
---
- A raw inventory read stopped by its flag keeps what it read: `raw::InventoryHalt::Cancelled` carries the inventory so far, its records read whole and the rest listed as unread, as a device going silent leaves them (#2620).
