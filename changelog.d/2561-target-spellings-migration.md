---
section: Migration notes
---
- **Targets (Python API, #2561):** `device()` reads the target grammar, not `address` forms. `ip:port`, six hex octets, `bip6:` forms and `mstp:N` mean the same; a bare `"5"` is device instance 5, not MS/TP station 5 (write `"mstp:5"`), and other colon-hex lengths are refused or, as `"10:20"`, read as network and instance.
