---
section: Migration notes
---
- **Breaking (Python API).** Handle duplicate endpoint object names or identifiers at the `add_*` call. Earlier registrations remain available for startup. Supply a finite initial Analog Input value and use instances 0 through 4194302 (#2688).
