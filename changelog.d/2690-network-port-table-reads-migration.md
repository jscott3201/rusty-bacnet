---
section: Migration notes
---
- **Network Port tables (Python API, #2690):** reads of properties 414 and 415 on a Network Port give `bdt` or `fdt` in place of `tagged` or an empty `list`. Read the entries from `.value`: `{"address", "broadcast_mask"}` mappings for the BDT and `FdtEntry` mappings for the FDT. Compare wire bytes with `.octets`.
