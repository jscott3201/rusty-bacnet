---
section: Added
---
- `bacnet-types`: the new `qudt` feature adds `bacnet_types::qudt`, the table of BACnet engineering units as QUDT unit and quantity-kind local names, with `qudt_unit` and the reverse `engineering_units_from_qudt`. It is const data with no allocation and no dependency (#2448).
