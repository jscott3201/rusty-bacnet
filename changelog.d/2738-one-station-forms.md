---
section: Added
---
- **Wire:** an address marked one station takes a confirmed request at the
  one octet `0xFF` (ARCNET's station 255) and at two octets starting `0x00`
  or `0xFF` (a gateway's virtual station), but not `0x0000`, `0xFFFF` or the
  one octet `0x00`; refusing an unmarked one names the mark (#2738).
