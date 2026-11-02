---
section: Added
---
- **Wire:** A B/IP foreign device's FD_BBMD_Address may name its BBMD by host name, looked up at activation and alongside later registrations; a failed lookup is counted and keeps the last address, and `BipTransport::set_bbmd_resolver` replaces the system resolver (#1852).
