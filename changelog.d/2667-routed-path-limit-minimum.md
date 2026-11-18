---
section: Changed
---
- **Breaking (Rust API):** `configure_routed_path_max_npdu` refuses a limit that leaves no request the 50-octet minimum APDU after the routed header (under 66 on B/IP, 61 on MS/TP) with `Error::Encoding`; such a limit failed every request before (#2667).
