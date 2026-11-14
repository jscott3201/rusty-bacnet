---
section: Fixed
---
- `bacnet_revert` checks the policy by the address each entry's write went
  to, so a network the server leaves out is refused, and it never sends a
  Who-Is: an entry whose device it can't place is reported not reverted
  (#2560).
