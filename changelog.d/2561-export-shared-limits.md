---
section: Changed
---
- **Python API:** `export_223_experimental` now shares the client's request limits with the `Device` calls, so their requests to one device go one at a time through the same per-device gate (#2561).
