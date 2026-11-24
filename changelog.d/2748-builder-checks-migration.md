---
section: Migration notes
---
- **Python API (#2748).** Register each object once: a repeated identifier raises `ValueError` (it used to replace the first), and there is no removal call before start, so build the server again to change one. A duplicate name raises at the call, not `start()`.
