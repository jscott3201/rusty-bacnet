---
section: Changed
---
- **Python API:** a whole read of a typed constructed list or array whose application-tagged octets don't decode as its elements reads as `application_data` with the octets as served, not as a re-encoded `list`, so a write-back sends the same octets (#2690).
