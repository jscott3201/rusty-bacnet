---
section: Fixed
---
- `bacnet_revert` checks the policy at the address where it finds each
  entry's device, so a network the server leaves out is refused, an
  attached revert goes only to the address confirmed, and it never sends a
  Who-Is: an entry it can't place is reported not reverted (#2560).
