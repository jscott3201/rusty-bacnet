---
section: Added
---
- **Wire:** an address marked one station takes a confirmed request at the
  one octet `0xFF` (ARCNET's station 255) and at two octets starting `0x00`
  or `0xFF` (a gateway's virtual station), but not `0x0000` or `0xFFFF`; the
  mark only lifts, and refusing an unmarked one names it (#2738).
